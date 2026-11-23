//! Shells and links, which are what make an installer script runnable.

use std::path::{Path, PathBuf};

use dome_image::initramfs::{Entry, Initramfs};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn static_init() -> Option<PathBuf> {
    let path = workspace_root().join("target/x86_64-unknown-linux-musl/release/dome-init");
    path.is_file().then_some(path)
}

fn scratch(name: &str) -> PathBuf {
    let unique = format!("dome-shell-{name}-{}", std::process::id());
    let dir = std::env::temp_dir().join(unique);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch");
    dir
}

#[test]
fn a_symlink_is_a_link_not_a_copy() {
    let mut image = Initramfs::new();
    image
        .add(Entry::executable("bin/bash", b"binary".to_vec()))
        .expect("bash");
    image.add(Entry::symlink("bin/sh", "bash")).expect("link");

    let cpio = image.to_cpio();

    // The target is the entry's data, and the mode says it is a link: a builder that copied the
    // binary would double the image and could ship a stale one.
    assert!(
        cpio.windows(4).any(|window| window == b"bash"),
        "the target is recorded"
    );
    let mode = 0o120_777u32;
    let hex = format!("{mode:08x}");
    assert!(
        cpio.windows(8).any(|window| window == hex.as_bytes()),
        "the mode marks the entry as a symlink"
    );
}

#[test]
fn a_statically_linked_shell_can_be_installed_as_a_profile() {
    let Some(shell) = static_init() else { return };

    let dir = scratch("profile");
    let root = dir.join("shells");

    let report = dome_cli::shell::import(&root, "bash", &shell).expect("import");
    assert!(report.contains("installed shell profile"), "{report}");

    assert_eq!(
        dome_cli::shell::list(&root).expect("list"),
        vec!["bash".to_owned()]
    );
    let resolved = dome_cli::shell::resolve(&root, "bash").expect("resolve");
    assert!(resolved.is_file());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_dynamic_shell_is_refused() {
    let dir = scratch("dynamic");
    let dynamic = dir.join("sh");
    // An ELF with PT_INTERP: dynamically linked.
    let mut bytes = vec![0u8; 128];
    bytes[0..4].copy_from_slice(b"\x7fELF");
    bytes[4] = 2;
    bytes[5] = 1;
    bytes[32..40].copy_from_slice(&64u64.to_le_bytes());
    bytes[54..56].copy_from_slice(&56u16.to_le_bytes());
    bytes[56..58].copy_from_slice(&1u16.to_le_bytes());
    bytes[64..68].copy_from_slice(&3u32.to_le_bytes());
    std::fs::write(&dynamic, bytes).expect("write");

    let error = dome_cli::shell::import(&dir.join("shells"), "sh", &dynamic).unwrap_err();
    assert!(error.contains("must be static"), "{error}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_missing_shell_profile_says_how_to_install_one() {
    let dir = scratch("missing");
    let error = dome_cli::shell::resolve(&dir.join("shells"), "bash").unwrap_err();
    assert!(error.contains("dome shell import"), "{error}");
    let _ = std::fs::remove_dir_all(&dir);
}
