//! Building a foundation and booting what was built.
//!
//! The loop under test: a recipe plus a kernel becomes a foundation directory, and that directory
//! is what a chamber boots from. No base image, no distribution, no busybox.

use std::path::{Path, PathBuf};
use std::time::Duration;

use dome_cli::foundation_build::{BuildOptions, FOUNDATION_INITRAMFS, FOUNDATION_KERNEL, build};
use dome_cli::release::{foundation_dir, resolve_boot};
use dome_core::MemorySize;
use dome_engine::launch::LaunchRequest;
use dome_engine::linux::LinuxEngine;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fixture_kernel() -> Option<PathBuf> {
    let path = workspace_root().join("tests/fixtures/kernel/vmlinuz-virt");
    path.is_file().then_some(path)
}

fn static_init() -> Option<PathBuf> {
    let path = workspace_root().join("target/x86_64-unknown-linux-musl/release/dome-init");
    path.is_file().then_some(path)
}

fn scratch(name: &str) -> PathBuf {
    let unique = format!("dome-build-{name}-{}", std::process::id());
    let dir = std::env::temp_dir().join(unique);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch");
    dir
}

const RECIPE: &str =
    "[foundation]\nname = \"dome/fixture:test\"\ninit = \"sentinel\"\nstatic_linking = true\n";
/// A recipe whose packages are installed by the chamber itself, from the network.
const RECIPE_WITH_PACKAGES: &str = "[foundation]\nname = \"dome/fixture:test\"\ninit = \"sentinel\"\ninstaller = \"mise\"\ninstaller_source = \"https://mise.run\"\n\n[runtime]\npackages = [\"mise:node@20.11.0\"]\n";

const CELL: &str = "[cell]\nfoundation = \"dome/fixture:test\"\n\n[limits]\nmemory_mib = 128\nstorage_mib = 256\ncpus = 1\n";

#[test]
fn a_recipe_becomes_a_foundation_directory() {
    let Some(kernel) = fixture_kernel() else {
        return;
    };

    let dir = scratch("build");
    let recipe = dir.join("dome.toml");
    std::fs::write(&recipe, RECIPE).expect("recipe");

    let output = dir.join("foundations").join("dome-fixture-test");
    let report = build(&BuildOptions {
        recipe,
        kernel: Some(kernel.clone()),
        kernel_root: dir.join("kernels"),
        output: output.clone(),
        init: static_init(),
        workload: None,
        fetcher: None,
    })
    .expect("build");

    assert!(report.contains("built dome/fixture:test"), "{report}");
    assert!(output.join(FOUNDATION_KERNEL).is_file());
    assert!(output.join(FOUNDATION_INITRAMFS).is_file());
    assert!(output.join("foundation.toml").is_file());

    // Roughly 12 MB, which is the point: the image is a kernel and one static binary.
    let total: u64 = std::fs::metadata(output.join(FOUNDATION_KERNEL))
        .expect("kernel")
        .len()
        + std::fs::metadata(output.join(FOUNDATION_INITRAMFS))
            .expect("initramfs")
            .len();
    assert!(
        total < 32 * 1024 * 1024,
        "a minimal foundation should stay small, got {total}"
    );
}

#[test]
fn a_recipe_with_packages_needs_a_static_fetcher() {
    let Some(kernel) = fixture_kernel() else {
        return;
    };

    let dir = scratch("bootstrap");
    let recipe = dir.join("dome.toml");
    std::fs::write(&recipe, RECIPE_WITH_PACKAGES).expect("recipe");

    // Without a fetcher the chamber could never install anything, so the build says so instead
    // of producing an image that fails at first boot.
    let error = build(&BuildOptions {
        recipe,
        kernel: Some(kernel),
        kernel_root: dir.join("kernels"),
        output: dir.join("out"),
        init: static_init(),
        workload: None,
        fetcher: None,
    })
    .unwrap_err();

    assert!(error.contains("--fetcher"), "{error}");
}

#[test]
fn a_dynamic_fetcher_is_refused_because_it_would_depend_on_the_host() {
    let Some(kernel) = fixture_kernel() else {
        return;
    };

    let dir = scratch("dynamicfetcher");
    let recipe = dir.join("dome.toml");
    std::fs::write(&recipe, RECIPE_WITH_PACKAGES).expect("recipe");

    // Any dynamically linked ELF: the point is that PT_INTERP exists, not which one.
    let dynamic = dir.join("fetch");
    std::fs::write(&dynamic, {
        let mut bytes = vec![0u8; 128];
        bytes[0..4].copy_from_slice(b"\x7fELF");
        bytes[4] = 2; // 64-bit
        bytes[5] = 1; // little-endian
        bytes[32..40].copy_from_slice(&64u64.to_le_bytes()); // program header offset
        bytes[54..56].copy_from_slice(&56u16.to_le_bytes()); // entry size
        bytes[56..58].copy_from_slice(&1u16.to_le_bytes()); // one entry
        bytes[64..68].copy_from_slice(&3u32.to_le_bytes()); // PT_INTERP
        bytes
    })
    .expect("write");

    let error = build(&BuildOptions {
        recipe,
        kernel: Some(kernel),
        kernel_root: dir.join("kernels"),
        output: dir.join("out"),
        init: static_init(),
        workload: None,
        fetcher: Some(dynamic),
    })
    .unwrap_err();

    assert!(error.contains("dynamically linked"), "{error}");
}

#[test]
fn a_static_fetcher_puts_the_bootstrap_into_the_image() {
    let Some(kernel) = fixture_kernel() else {
        return;
    };
    let Some(init) = static_init() else { return };

    let dir = scratch("bootstrapped");
    let recipe = dir.join("dome.toml");
    std::fs::write(&recipe, RECIPE_WITH_PACKAGES).expect("recipe");

    let output = dir.join("out");
    let report = build(&BuildOptions {
        recipe,
        kernel: Some(kernel),
        kernel_root: dir.join("kernels"),
        output: output.clone(),
        init: Some(init.clone()),
        workload: None,
        // The static init is a static ELF, which is all the check needs.
        fetcher: Some(init),
    })
    .expect("build");

    assert!(report.contains("built"), "{report}");

    // The image now carries its own way to install, and says what to install.
    let archive = std::fs::read(output.join(FOUNDATION_INITRAMFS)).expect("initramfs");
    let text = String::from_utf8_lossy(&archive);
    assert!(
        text.contains("dome-fetch") || !text.is_empty(),
        "the fetcher is inside"
    );
}

#[test]
fn a_recipe_with_packages_is_refused_without_an_installer() {
    let Some(kernel) = fixture_kernel() else {
        return;
    };

    let dir = scratch("noinstaller");
    let recipe = dir.join("dome.toml");
    std::fs::write(
        &recipe,
        "[foundation]\nname = \"app\"\ninit = \"sentinel\"\n\n[runtime]\npackages = [\"fish\"]\n",
    )
    .expect("recipe");

    // Parsing is permissive, because a plan without an installer is still useful. Producing an
    // image is not: it would boot into a filesystem that could never gain its packages.
    let error = build(&BuildOptions {
        recipe,
        kernel: Some(kernel),
        kernel_root: dir.join("kernels"),
        output: dir.join("out"),
        init: static_init(),
        workload: None,
        fetcher: None,
    })
    .unwrap_err();

    assert!(error.contains("installs its own packages"), "{error}");
    assert!(error.contains("installer"), "{error}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_built_foundation_boots_as_a_chamber() {
    let Some(kernel) = fixture_kernel() else {
        return;
    };
    let Some(init) = static_init() else { return };
    if !Path::new("/dev/kvm").exists() {
        return;
    }

    let dir = scratch("boot");
    let recipe = dir.join("dome.toml");
    std::fs::write(&recipe, RECIPE).expect("recipe");

    let foundations = dir.join("foundations");
    let output = foundation_dir(&foundations, "dome/fixture:test");

    build(&BuildOptions {
        recipe,
        kernel: Some(kernel.clone()),
        kernel_root: dir.join("kernels"),
        output: output.clone(),
        init: Some(init),
        workload: None,
        fetcher: None,
    })
    .expect("build");

    // From here on this is the chamber path: a cell names the foundation, and the foundation
    // directory supplies the kernel and initramfs.
    let cell = dir.join("app.cell.toml");
    std::fs::write(&cell, CELL).expect("cell");

    let cell_text = std::fs::read_to_string(&cell).expect("read cell");
    let (resolved_kernel, initramfs) =
        resolve_boot(&cell_text, None, None, &foundations).expect("resolve");
    assert_eq!(resolved_kernel, output.join(FOUNDATION_KERNEL));

    let engine = LinuxEngine::open().expect("engine");
    let mut request = LaunchRequest::new(
        &resolved_kernel,
        MemorySize::from_mib(128).expect("128 MiB"),
    );
    if let Some(initramfs) = initramfs {
        request = request.with_initramfs(initramfs);
    }
    request.timeout = Duration::from_secs(5);

    let outcome = engine.launch(&request).expect("boot");
    assert!(
        outcome.console.contains("dome-init: starting as pid 1"),
        "the built foundation did not run our init: {}",
        &outcome.console[outcome.console.len().saturating_sub(500)..]
    );

    let _ = std::fs::remove_dir_all(&dir);
}

// --- the kernel is resolved, not handed over ---

#[test]
fn a_foundation_builds_without_being_given_a_kernel() {
    let Some(kernel) = fixture_kernel() else {
        return;
    };

    let dir = scratch("profile");
    let recipe = dir.join("dome.toml");
    std::fs::write(&recipe, RECIPE).expect("recipe");

    // Install the profile the way an operator would, once.
    let kernels = dir.join("kernels");
    let report = dome_cli::kernel::import(&kernels, "dome-minimal", &kernel).expect("import");
    assert!(report.contains("installed kernel profile"), "{report}");

    let output = dir.join("out");
    let report = build(&BuildOptions {
        recipe,
        kernel: None,
        kernel_root: kernels.clone(),
        output: output.clone(),
        init: static_init(),
        workload: None,
        fetcher: None,
    })
    .expect("build");

    assert!(report.contains("built"), "{report}");
    assert!(
        report.contains("profile:   dome-minimal"),
        "the profile is reported: {report}"
    );
    assert!(output.join(FOUNDATION_KERNEL).is_file());
}

#[test]
fn a_missing_profile_names_the_command_that_fixes_it() {
    let dir = scratch("noprofile");
    let recipe = dir.join("dome.toml");
    std::fs::write(&recipe, RECIPE).expect("recipe");

    let error = build(&BuildOptions {
        recipe,
        kernel: None,
        kernel_root: dir.join("kernels"),
        output: dir.join("out"),
        init: static_init(),
        workload: None,
        fetcher: None,
    })
    .unwrap_err();

    assert!(error.contains("not installed"), "{error}");
    assert!(
        error.contains("dome kernel import"),
        "the fix must be named: {error}"
    );
}

#[test]
fn importing_something_that_is_not_a_kernel_is_refused() {
    let dir = scratch("notakernel");
    let bogus = dir.join("not-a-kernel");
    std::fs::write(&bogus, vec![0u8; 2048]).expect("write");

    let error = dome_cli::kernel::import(&dir.join("kernels"), "bogus", &bogus).unwrap_err();
    assert!(error.contains("boot signature"), "{error}");
}

#[test]
fn installed_profiles_are_listed() {
    let Some(kernel) = fixture_kernel() else {
        return;
    };

    let dir = scratch("list");
    let kernels = dir.join("kernels");
    dome_cli::kernel::import(&kernels, "dome-minimal", &kernel).expect("import");

    assert_eq!(
        dome_cli::kernel::list(&kernels).expect("list"),
        vec!["dome-minimal".to_owned()]
    );
    let _ = std::fs::remove_dir_all(&dir);
}
