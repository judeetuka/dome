//! Export and import: a package that travels and comes back usable.

use std::path::{Path, PathBuf};
use std::time::Duration;

use dome_cli::packaging::{export, import};
use dome_cli::release::{KeyFiles, ReleaseOptions, inspect, key_file_for, release};

fn scratch(name: &str) -> PathBuf {
    let unique = format!("dome-export-{name}-{}", std::process::id());
    let dir = std::env::temp_dir().join(unique);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch");
    dir
}

fn fixture_kernel() -> Option<PathBuf> {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/kernel/vmlinuz-virt");
    path.is_file().then_some(path)
}

fn kvm_available() -> bool {
    Path::new("/dev/kvm").exists()
}

const CELL: &str = "[cell]\nfoundation = \"dome/fixture:test\"\n\n[limits]\nmemory_mib = 64\nstorage_mib = 256\ncpus = 1\n";

#[test]
fn a_package_round_trips_into_a_usable_artifact() {
    let Some(kernel) = fixture_kernel() else {
        return;
    };
    if !kvm_available() {
        return;
    }

    let dir = scratch("roundtrip");
    let keys = KeyFiles::in_dir(dir.join("keys"));
    keys.generate().expect("keygen");

    let cell = dir.join("example.cell.toml");
    std::fs::write(&cell, CELL).expect("cell");

    let artifact = dir.join("app.dome");
    let mut options = ReleaseOptions::new(&cell, &artifact, keys.clone());
    options.kernel = Some(kernel);
    options.memory_mib = 64;
    options.timeout = Duration::from_secs(3);
    release(&options).expect("release");

    let before = inspect(&artifact, &keys).expect("inspect the original");

    let package = dir.join("app.export.dome");
    let report = export(&artifact, &keys, &package).expect("export");
    assert!(report.contains("exported"), "{report}");
    assert!(package.is_file(), "the package was not written");
    assert!(
        key_file_for(&package).is_file(),
        "the package key was not written"
    );

    let imported = dir.join("imported.dome");
    let report = import(&package, &keys, &imported).expect("import");
    assert!(report.contains("imported"), "{report}");
    assert!(imported.is_file(), "the artifact was not restored");

    let after = inspect(&imported, &keys).expect("inspect the imported artifact");

    // The round trip must preserve what the artifact claims, or the package changed it.
    let field = |text: &str, name: &str| -> String {
        text.lines()
            .find(|line| line.trim_start().starts_with(name))
            .map(|line| line.trim().to_owned())
            .unwrap_or_default()
    };

    assert_eq!(
        field(&before, "distribution:"),
        field(&after, "distribution:")
    );
    assert_eq!(field(&before, "chamber:"), field(&after, "chamber:"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn exporting_without_the_artifact_fails_clearly() {
    let dir = scratch("nokeys");
    let keys = KeyFiles::in_dir(dir.join("keys"));
    keys.generate().expect("keygen");

    let missing = dir.join("absent.dome");
    let error = export(&missing, &keys, &dir.join("out.dome")).unwrap_err();

    assert!(error.contains("cannot read"), "{error}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn importing_something_that_is_not_a_package_fails() {
    let dir = scratch("notapackage");
    let keys = KeyFiles::in_dir(dir.join("keys"));
    keys.generate().expect("keygen");

    let bogus = dir.join("bogus.dome");
    std::fs::write(&bogus, b"not a package at all").expect("write");

    let error = import(&bogus, &keys, &dir.join("out.dome")).unwrap_err();
    assert!(!error.is_empty(), "the failure must explain itself");

    let _ = std::fs::remove_dir_all(&dir);
}
