//! Building a kernel the same way, every time.
//!
//! A kernel that differs because someone ran menuconfig in a terminal is a kernel nobody can
//! reproduce. This command is the whole procedure: fetch the source, apply the checked-in
//! fragment, build, install as a profile.
//!
//! It is deliberately boring. There are no options for the toolchain, because a kernel built with
//! a different compiler is a different kernel, and a build that silently picks up whatever is on
//! PATH is a build that cannot be repeated.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::kernel;

/// Tools a kernel build needs, and the package that provides each on Arch.
pub const REQUIRED_TOOLS: [(&str, &str); 6] = [
    ("make", "base-devel"),
    ("gcc", "base-devel"),
    ("bc", "bc"),
    ("flex", "flex"),
    ("bison", "bison"),
    ("pahole", "pahole"),
];

/// What to build.
#[derive(Debug, Clone)]
pub struct BuildOptions {
    /// Kernel version, for example 7.3.6.
    pub version: String,
    /// The profile name to install under.
    pub profile: String,
    /// Parallel jobs; defaults to the machine's core count.
    pub jobs: Option<u32>,
    /// Where sources and tarballs live.
    pub cache: PathBuf,
    /// The configuration fragment, checked in beside the code.
    pub fragment: PathBuf,
}

/// Where a version's tarball lives on kernel.org.
///
/// The directory is by major series (v7.x), which is why a version is not just a suffix.
pub fn tarball_url(version: &str) -> Result<String, String> {
    let mut parts = version.split('.');

    let major = parts.next().unwrap_or_default();
    if major.is_empty() || !major.chars().all(|character| character.is_ascii_digit()) {
        return Err(format!("{version:?} is not a kernel version"));
    }

    // A bare major ("7") names no release, and a trailing dot names no minor: both would be
    // fetched as a 404 that reads like a missing version rather than a bad input.
    if parts.next().is_none_or(str::is_empty) || version.ends_with('.') {
        return Err(format!("{version:?} is not a kernel version"));
    }

    Ok(format!(
        "https://cdn.kernel.org/pub/linux/kernel/v{major}.x/linux-{version}.tar.xz"
    ))
}

/// Which required tools are missing.
pub fn missing_tools() -> Vec<(&'static str, &'static str)> {
    REQUIRED_TOOLS
        .iter()
        .filter(|(tool, _)| !on_path(tool))
        .copied()
        .collect()
}

fn on_path(tool: &str) -> bool {
    std::env::var_os("PATH")
        .and_then(|paths| {
            std::env::split_paths(&paths).find_map(|directory| {
                let candidate = directory.join(tool);
                candidate.is_file().then_some(())
            })
        })
        .is_some()
}

/// Build a kernel and install it as a profile.
pub fn build(options: &BuildOptions) -> Result<String, String> {
    let missing = missing_tools();
    if !missing.is_empty() {
        let list: Vec<String> = missing
            .iter()
            .map(|(tool, package)| format!("{tool} ({package})"))
            .collect();

        return Err(format!(
            "missing tools for a kernel build: {}. Install them with: pacman -S --needed {}",
            list.join(", "),
            missing
                .iter()
                .map(|(_, package)| *package)
                .collect::<Vec<_>>()
                .join(" ")
        ));
    }

    std::fs::create_dir_all(&options.cache)
        .map_err(|error| format!("cannot create {}: {error}", options.cache.display()))?;

    let tarball = options
        .cache
        .join(format!("linux-{}.tar.xz", options.version));
    let source = options.cache.join(format!("linux-{}", options.version));

    if !source.is_dir() {
        if !tarball.is_file() {
            let url = tarball_url(&options.version)?;
            fetch(&url, &tarball)?;
        }

        let status = Command::new("tar")
            .args(["xf", tarball.to_str().unwrap_or_default()])
            .current_dir(&options.cache)
            .status()
            .map_err(|error| format!("cannot run tar: {error}"))?;

        if !status.success() {
            return Err(format!("tar failed for {}", tarball.display()));
        }
    }

    // Configure once. A second run continues an existing configuration, which is what makes the
    // build resumable: a kernel build is minutes long and may be interrupted.
    let config = source.join(".config");
    if !config.is_file() {
        run(&source, &["defconfig"], "defconfig")?;

        let fragment = std::fs::read_to_string(&options.fragment)
            .map_err(|error| format!("cannot read {}: {error}", options.fragment.display()))?;
        // Appending then olddefconfig is how Kbuild's own merge script does it: the file is
        // the input, and Kbuild resolves dependencies and drops what cannot be set.
        let mut existing = std::fs::read_to_string(&config)
            .map_err(|error| format!("cannot read the generated config: {error}"))?;
        existing.push('\n');
        existing.push_str(&fragment);
        std::fs::write(&config, existing)
            .map_err(|error| format!("cannot write the config: {error}"))?;

        run(&source, &["olddefconfig"], "olddefconfig")?;
    }

    let jobs = options.jobs.unwrap_or_else(|| {
        std::thread::available_parallelism()
            .map(|count| count.get() as u32)
            .unwrap_or(1)
    });

    // KCFLAGS rather than patched Makefiles: GCC 16 defaults to C23 and this kernel predates it,
    // and WERROR=0 because a warning from a newer compiler is not a reason to refuse to build.
    run(
        &source,
        &[
            "-j",
            &jobs.to_string(),
            "WERROR=0",
            "KCFLAGS=-std=gnu11",
            "bzImage",
        ],
        "make bzImage",
    )?;

    let image = source.join("arch/x86/boot/bzImage");
    let size = std::fs::metadata(&image)
        .map_err(|error| format!("no kernel image at {}: {error}", image.display()))?
        .len();

    let report = kernel::import(&kernel::kernel_root(), &options.profile, &image)?;

    Ok(format!(
        "built Linux {version}\n  source: {source}\n  image:  {size} bytes\n{report}",
        version = options.version,
        source = source.display(),
        size = size,
        report = report
    ))
}

fn fetch(url: &str, destination: &Path) -> Result<(), String> {
    let status = Command::new("curl")
        .args([
            "-fSL",
            "--retry",
            "3",
            "-o",
            destination.to_str().unwrap_or_default(),
            url,
        ])
        .status()
        .map_err(|error| format!("cannot run curl: {error}"))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!("cannot fetch {url}"))
    }
}

fn run(directory: &Path, arguments: &[&str], what: &str) -> Result<(), String> {
    let status = Command::new("make")
        .args(arguments)
        .current_dir(directory)
        .status()
        .map_err(|error| format!("cannot run {what}: {error}"))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!("{what} failed"))
    }
}
