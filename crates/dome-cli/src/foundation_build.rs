//! Building a foundation from a recipe.
//!
//! A foundation is a kernel, an initramfs, and metadata. There is no base image and no
//! distribution: the archive holds the kernel's first filesystem, which for a minimal image is
//! one static binary.
//!
//! What this does not do yet is install packages. That needs a package manager on the host
//! writing into a staging root, which is the next piece of M5; a recipe that asks for packages is
//! refused rather than silently producing an image without them.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use dome_config::foundation_recipe::Recipe;
use dome_config::include::load_merged;
use dome_image::initramfs::{Entry, Initramfs};

/// The file a foundation ships as its kernel.
pub const FOUNDATION_KERNEL: &str = "vmlinuz";
/// The file a foundation ships as its initramfs.
pub const FOUNDATION_INITRAMFS: &str = "initramfs.cpio.gz";

/// Where a workload embedded in an image is placed.
pub const EMBEDDED_WORKLOAD: &str = "workload";

/// What to build.
#[derive(Debug, Clone)]
pub struct BuildOptions {
    /// The recipe, which may include others.
    pub recipe: PathBuf,
    /// A kernel image to use instead of the recipe's profile. A development override.
    pub kernel: Option<PathBuf>,
    /// Where kernel profiles live. Overridable so tests do not touch the real store.
    pub kernel_root: PathBuf,
    /// Where the foundation directory goes.
    pub output: PathBuf,
    /// The static init to run as pid 1. Defaults to the musl build in the workspace.
    pub init: Option<PathBuf>,
    /// A program to embed and run, if the image exists to run one.
    pub workload: Option<PathBuf>,
}

/// Build a foundation directory and report what it contains.
pub fn build(options: &BuildOptions) -> Result<String, String> {
    let merged = load_merged(&options.recipe).map_err(|error| error.to_string())?;
    let recipe = Recipe::from_toml(&merged).map_err(|error| error.to_string())?;
    let plan = recipe.plan();

    // Refusing is the honest option: an image that silently contains none of the packages it
    // lists would look like a successful build and fail at run time.
    if !recipe.runtime.packages.is_empty() {
        return Err(format!(
            "this recipe asks for {} runtime package(s) ({}), and package installation is not \
             implemented yet: it needs apk or pacman writing into a staging root. Build a \
             package-free recipe, or wait for the package stage.",
            recipe.runtime.packages.len(),
            plan.runtime_packages.join(", ")
        ));
    }

    let init = match options.init.clone() {
        Some(path) => path,
        None => default_init().ok_or_else(|| {
            "no static init found. Build one with: cargo build --release --target \
             x86_64-unknown-linux-musl -p dome-init"
                .to_owned()
        })?,
    };

    // The recipe names a profile, not a path: a user should no more pass a kernel than they
    // should pass a C library.
    let kernel = match options.kernel.clone() {
        Some(path) => path,
        None => crate::kernel::resolve(&options.kernel_root, &recipe.kernel)?,
    };

    let init_bytes = fs::read(&init)
        .map_err(|error| format!("cannot read the init at {}: {error}", init.display()))?;

    let mut image = Initramfs::new();
    for directory in ["dev", "proc", "sys", "tmp"] {
        image
            .add(Entry::directory(directory))
            .map_err(|error| error.to_string())?;
    }

    image
        .add(Entry::executable("init", init_bytes))
        .map_err(|error| error.to_string())?;

    if let Some(workload) = options.workload.as_ref() {
        let bytes = fs::read(workload)
            .map_err(|error| format!("cannot read the workload {}: {error}", workload.display()))?;
        image
            .add(Entry::executable(EMBEDDED_WORKLOAD, bytes))
            .map_err(|error| error.to_string())?;
    }

    let archive = image.to_gzip().map_err(|error| error.to_string())?;

    fs::create_dir_all(&options.output)
        .map_err(|error| format!("cannot create {}: {error}", options.output.display()))?;

    let kernel_bytes = fs::read(&kernel)
        .map_err(|error| format!("cannot read the kernel {}: {error}", kernel.display()))?;

    let kernel_path = options.output.join(FOUNDATION_KERNEL);
    let initramfs_path = options.output.join(FOUNDATION_INITRAMFS);

    fs::write(&kernel_path, &kernel_bytes)
        .map_err(|error| format!("cannot write {}: {error}", kernel_path.display()))?;
    fs::write(&initramfs_path, &archive)
        .map_err(|error| format!("cannot write {}: {error}", initramfs_path.display()))?;

    // A small manifest so the directory explains itself without a database.
    let meta = format!(
        "name={}\narch={}\ninit={}\nkernel={}\ncreated={}\nstatic_linking={}\n",
        recipe.name,
        recipe.arch,
        plan.kernel_cmdline,
        recipe.kernel,
        created_unix(),
        recipe.static_linking
    );
    fs::write(options.output.join("foundation.toml"), meta)
        .map_err(|error| format!("cannot write the manifest: {error}"))?;

    let mut report = String::new();
    report.push_str(&format!(
        "built {} into {}\n",
        recipe.name,
        options.output.display()
    ));
    report.push_str(&format!("  kernel:    {} bytes\n", kernel_bytes.len()));
    report.push_str(&format!("  initramfs: {} bytes\n", archive.len()));
    report.push_str(&format!("  init:      {}\n", init.display()));
    report.push_str(&format!("  profile:   {}\n", recipe.kernel));
    report.push_str(&format!(
        "  total:     {} bytes\n",
        kernel_bytes.len() + archive.len()
    ));
    for note in plan.notes {
        report.push_str(&format!("  note: {note}\n"));
    }

    Ok(report)
}

/// The static init shipped with the workspace, if it has been built.
pub fn default_init() -> Option<PathBuf> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let path = workspace.join("target/x86_64-unknown-linux-musl/release/dome-init");

    path.is_file().then_some(path)
}

fn created_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|delta| delta.as_secs())
        .unwrap_or(0)
}
