//! The release loop: keygen, build, inspect.
//!
//! These boot a real kernel to capture state, so they skip when there is no /dev/kvm rather
//! than pretending to have run.

use std::path::{Path, PathBuf};
use std::time::Duration;

use dome_cli::release::{
    KeyFiles, ReleaseOptions, foundation_dir, inspect, key_file_for, release, resolve_boot,
};

fn scratch(name: &str) -> PathBuf {
    let unique = format!("dome-release-{name}-{}", std::process::id());
    let dir = std::env::temp_dir().join(unique);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn kernel() -> Option<PathBuf> {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/kernel/vmlinuz-virt");
    path.is_file().then_some(path)
}

fn kvm_available() -> bool {
    Path::new("/dev/kvm").exists()
}

fn write_cell(dir: &Path) -> PathBuf {
    let path = dir.join("example.cell.toml");
    std::fs::write(
        &path,
        // memory_mib belongs under limits: release now parses the blueprint, so an invalid
        // cell is caught here rather than producing an artifact nobody can boot.
        "[cell]\nfoundation = \"dome/alpine:3.20\"\n\n[limits]\nmemory_mib = 64\nstorage_mib = 256\ncpus = 1\n",
    )
    .expect("cell");
    path
}

#[test]
fn keygen_writes_every_key_and_restricts_the_secrets() {
    let dir = scratch("keygen");
    let keys = KeyFiles::in_dir(dir.join("keys"));

    keys.generate().expect("keygen");
    keys.seal_permissions().expect("permissions");

    for path in [
        &keys.identity,
        &keys.recipient,
        &keys.signing,
        &keys.signing_public,
        &keys.mldsa,
        &keys.mldsa_public,
    ] {
        assert!(path.is_file(), "{} was not written", path.display());
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&keys.identity)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "the identity must not be world readable");
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn keygen_refuses_to_overwrite_existing_keys() {
    let dir = scratch("overwrite");
    let keys = KeyFiles::in_dir(dir.join("keys"));

    keys.generate().expect("first keygen");
    let error = keys.generate().unwrap_err();

    assert!(error.contains("refusing to overwrite"), "{error}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_release_is_portable_by_default_and_runs_here() {
    let Some(kernel) = kernel() else { return };
    if !kvm_available() {
        return;
    }

    let dir = scratch("portable");
    let keys = KeyFiles::in_dir(dir.join("keys"));
    keys.generate().expect("keygen");

    let cell = write_cell(&dir);
    let artifact = dir.join("example.dome");

    let mut options = ReleaseOptions::new(&cell, &artifact, keys.clone());
    options.kernel = Some(kernel.clone());
    options.memory_mib = 64;
    options.timeout = Duration::from_secs(3);

    let report = release(&options).expect("release");
    assert!(report.contains("portable"), "{report}");
    assert!(artifact.is_file(), "the artifact was not written");
    assert!(
        key_file_for(&artifact).is_file(),
        "the key file was not written"
    );

    // The default is unsealed, so this device can run it.
    let described = inspect(&artifact, &keys).expect("inspect");
    assert!(described.contains("distribution: portable"), "{described}");
    assert!(described.contains("runs, with unprotected"), "{described}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_sealed_release_is_produced_here_but_cannot_run_here() {
    let Some(kernel) = kernel() else { return };
    if !kvm_available() {
        return;
    }

    let dir = scratch("sealed");
    let keys = KeyFiles::in_dir(dir.join("keys"));
    keys.generate().expect("keygen");

    let cell = write_cell(&dir);
    let artifact = dir.join("sealed.dome");

    let mut options = ReleaseOptions::new(&cell, &artifact, keys.clone());
    options.kernel = Some(kernel.clone());
    options.memory_mib = 64;
    options.timeout = Duration::from_secs(3);
    options.sealed = true;

    let report = release(&options).expect("any device may create a sealed release");
    assert!(report.contains("sealed"), "{report}");

    // Creation everywhere; execution only where the hardware can keep the promise.
    let described = inspect(&artifact, &keys).expect("inspect");
    assert!(described.contains("distribution: sealed"), "{described}");
    assert!(described.contains("cannot run it"), "{described}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn inspect_refuses_a_tampered_release() {
    let Some(kernel) = kernel() else { return };
    if !kvm_available() {
        return;
    }

    let dir = scratch("tampered");
    let keys = KeyFiles::in_dir(dir.join("keys"));
    keys.generate().expect("keygen");

    let cell = write_cell(&dir);
    let artifact = dir.join("example.dome");

    let mut options = ReleaseOptions::new(&cell, &artifact, keys.clone());
    options.kernel = Some(kernel.clone());
    options.memory_mib = 64;
    options.timeout = Duration::from_secs(3);
    release(&options).expect("release");

    // Flip a byte in the payload region.
    let mut bytes = std::fs::read(&artifact).expect("read");
    let last = bytes.len() - 1;
    bytes[last] ^= 0x01;
    std::fs::write(&artifact, &bytes).expect("write");

    let error = inspect(&artifact, &keys).unwrap_err();
    assert!(error.contains("cannot open the release"), "{error}");

    let _ = std::fs::remove_dir_all(&dir);
}

// --- the kernel comes from the foundation, not from the command line ---

const CELL_WITH_FOUNDATION: &str = "[cell]\nfoundation = \"dome/fixture:test\"\n\n[limits]\nmemory_mib = 64\nstorage_mib = 256\ncpus = 1\n";

#[test]
fn foundation_names_map_to_directories_predictably() {
    let root = Path::new("/foundations");

    // The mapping is a contract a build step will have to honour, so it is pinned.
    assert_eq!(
        foundation_dir(root, "dome/fixture:test"),
        root.join("dome-fixture-test")
    );
    assert_eq!(foundation_dir(root, "alpine"), root.join("alpine"));
}

#[test]
fn the_cell_supplies_the_kernel_when_none_is_given() {
    let dir = scratch("foundation-ok");
    let foundations = dir.join("foundations");
    let installed = foundation_dir(&foundations, "dome/fixture:test");
    std::fs::create_dir_all(&installed).expect("foundation dir");

    let source = kernel().expect("fixture kernel");
    std::fs::copy(&source, installed.join("vmlinuz")).expect("install kernel");

    let (resolved, initramfs) = resolve_boot(CELL_WITH_FOUNDATION, None, None, &foundations)
        .expect("the foundation should supply the kernel");

    assert_eq!(resolved, installed.join("vmlinuz"));
    assert!(initramfs.is_none(), "no initramfs was installed");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_installed_initramfs_is_picked_up_automatically() {
    let dir = scratch("foundation-initramfs");
    let foundations = dir.join("foundations");
    let installed = foundation_dir(&foundations, "dome/fixture:test");
    std::fs::create_dir_all(&installed).expect("foundation dir");

    std::fs::copy(kernel().expect("fixture kernel"), installed.join("vmlinuz")).expect("kernel");
    std::fs::write(installed.join("initramfs.cpio.gz"), b"not really a cpio").expect("initramfs");

    let (_, initramfs) =
        resolve_boot(CELL_WITH_FOUNDATION, None, None, &foundations).expect("resolve");

    assert_eq!(initramfs, Some(installed.join("initramfs.cpio.gz")));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_missing_foundation_names_what_is_expected() {
    let dir = scratch("foundation-missing");
    let foundations = dir.join("foundations");

    let error = resolve_boot(CELL_WITH_FOUNDATION, None, None, &foundations).unwrap_err();

    assert!(error.contains("is not installed"), "{error}");
    assert!(
        error.contains("dome-fixture-test"),
        "the error should name the directory: {error}"
    );
    assert!(
        error.contains("--kernel"),
        "the error should name the override: {error}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_explicit_kernel_still_wins() {
    let dir = scratch("foundation-override");
    let source = kernel().expect("fixture kernel");

    let (resolved, _) = resolve_boot(CELL_WITH_FOUNDATION, Some(&source), None, &dir.join("nope"))
        .expect("an explicit kernel needs no foundation");

    assert_eq!(resolved, source);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_release_needs_no_kernel_argument_when_the_foundation_is_installed() {
    let Some(source) = kernel() else { return };
    if !kvm_available() {
        return;
    }

    let dir = scratch("release-by-foundation");
    let keys = KeyFiles::in_dir(dir.join("keys"));
    keys.generate().expect("keygen");

    let foundations = dir.join("foundations");
    let installed = foundation_dir(&foundations, "dome/fixture:test");
    std::fs::create_dir_all(&installed).expect("foundation dir");
    std::fs::copy(&source, installed.join("vmlinuz")).expect("install kernel");

    let cell = dir.join("example.cell.toml");
    std::fs::write(&cell, CELL_WITH_FOUNDATION).expect("cell");

    let artifact = dir.join("by-foundation.dome");
    let mut options = ReleaseOptions::new(&cell, &artifact, keys.clone());
    options.foundation_root = foundations;
    options.memory_mib = 64;
    options.timeout = Duration::from_secs(3);

    let report = release(&options).expect("the cell alone should be enough");
    // The report names the kernel it resolved, which is how an operator sees which installed
    // foundation was used without having passed one.
    assert!(
        report.contains("dome-fixture-test"),
        "the report should name the foundation: {report}"
    );
    assert!(report.contains("kernel:"), "{report}");

    let described = inspect(&artifact, &keys).expect("inspect");
    assert!(described.contains("distribution: portable"), "{described}");

    let _ = std::fs::remove_dir_all(&dir);
}
