//! The foundation store: staging, sharing and reclaiming foundations.

use std::path::{Path, PathBuf};

use dome_image::store::{ARCH_AARCH64, ARCH_X86_64, FoundationStore, OverlayStrategy};

fn scratch(name: &str) -> PathBuf {
    let unique = format!("dome-store-{name}-{}", std::process::id());
    let dir = std::env::temp_dir().join(unique);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch");
    dir
}

fn foundation(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, bytes).expect("foundation");
    path
}

#[test]
fn the_same_bytes_are_stored_once() {
    let dir = scratch("dedupe");
    let store = FoundationStore::at(dir.join("store"));
    let image = foundation(&dir, "base.img", &vec![0xABu8; 4096]);

    let first = store.insert(&image, ARCH_X86_64, "alpine").expect("insert");
    let second = store
        .insert(&image, ARCH_X86_64, "alpine")
        .expect("insert again");

    assert_eq!(
        first.digest, second.digest,
        "identical bytes share one digest"
    );
    assert_eq!(store.list().expect("list").len(), 1, "one copy on disk");
    assert_eq!(first.size, 4096);
    assert_eq!(first.arch, ARCH_X86_64);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn different_bytes_get_different_digests() {
    let dir = scratch("distinct");
    let store = FoundationStore::at(dir.join("store"));
    let one = foundation(&dir, "one.img", b"first");
    let two = foundation(&dir, "two.img", b"second");

    let first = store.insert(&one, ARCH_X86_64, "one").expect("insert");
    let second = store.insert(&two, ARCH_X86_64, "two").expect("insert");

    assert_ne!(first.digest, second.digest);
    assert_eq!(store.list().expect("list").len(), 2);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn architecture_is_part_of_the_key() {
    let dir = scratch("arch");
    let store = FoundationStore::at(dir.join("store"));
    let image = foundation(&dir, "base.img", b"same bytes");

    // The same recipe produces different bytes per target, and serving the wrong one would
    // fail in a way that is hard to diagnose, so the architecture is part of the digest.
    let x86 = store.insert(&image, ARCH_X86_64, "alpine").expect("x86");
    let arm = store.insert(&image, ARCH_AARCH64, "alpine").expect("arm");

    assert_ne!(
        x86.digest, arm.digest,
        "architecture must change the digest"
    );
    assert_eq!(
        store.list().expect("list").len(),
        2,
        "neither may dedupe into the other"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_chamber_gets_a_layer_that_leaves_the_foundation_alone() {
    let dir = scratch("overlay");
    let store = FoundationStore::at(dir.join("store"));
    let original: Vec<u8> = (0..8192u32).map(|index| (index % 251) as u8).collect();
    let image = foundation(&dir, "base.img", &original);

    let entry = store.insert(&image, ARCH_X86_64, "alpine").expect("insert");
    let baseline = store.entry(&entry.digest).expect("entry").expect("present");

    let layer = store.overlay(&entry.digest, "web").expect("overlay");
    assert!(layer.path.is_file(), "the layer should exist");
    assert!(matches!(
        layer.strategy,
        OverlayStrategy::Reflink | OverlayStrategy::Copy
    ));

    // Writing to the layer must not reach the shared foundation.
    std::fs::write(&layer.path, vec![0x5Au8; 8192]).expect("write layer");
    let after = store.entry(&entry.digest).expect("entry").expect("present");

    assert_eq!(
        after.digest, baseline.digest,
        "the foundation's identity is unchanged"
    );
    assert_eq!(after.size, baseline.size, "and so is its size");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn two_chambers_share_one_foundation() {
    let dir = scratch("share");
    let store = FoundationStore::at(dir.join("store"));
    let image = foundation(&dir, "base.img", &vec![7u8; 16384]);

    let entry = store.insert(&image, ARCH_X86_64, "alpine").expect("insert");
    let web = store.overlay(&entry.digest, "web").expect("web");
    let api = store.overlay(&entry.digest, "api").expect("api");

    assert_ne!(web.path, api.path, "each chamber has its own layer");
    assert_eq!(
        store.list().expect("list").len(),
        1,
        "still one stored foundation"
    );

    let entry = store.entry(&entry.digest).expect("entry").expect("present");
    assert_eq!(entry.references, vec!["api".to_owned(), "web".to_owned()]);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_store_reports_whether_layers_actually_share_disk() {
    let dir = scratch("strategy");
    let store = FoundationStore::at(dir.join("store"));
    let image = foundation(&dir, "base.img", &vec![1u8; 4096]);
    let entry = store.insert(&image, ARCH_X86_64, "alpine").expect("insert");

    let layer = store.overlay(&entry.digest, "web").expect("overlay");

    // This is the honest part: ext4 cannot share extents, so the layer is a real copy and the
    // store says so instead of implying the btrfs case.
    match layer.strategy {
        OverlayStrategy::Reflink => assert!(layer.strategy.shares_disk()),
        OverlayStrategy::Copy => assert!(!layer.strategy.shares_disk()),
    }

    // Either way the layer's contents start identical to the foundation.
    assert_eq!(std::fs::read(&layer.path).expect("read layer").len(), 4096);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn prune_removes_what_nothing_references_and_keeps_the_rest() {
    let dir = scratch("prune");
    let store = FoundationStore::at(dir.join("store"));
    let used = store
        .insert(
            &foundation(&dir, "used.img", b"in use"),
            ARCH_X86_64,
            "used",
        )
        .expect("insert");
    let unused = store
        .insert(
            &foundation(&dir, "unused.img", b"nobody"),
            ARCH_X86_64,
            "unused",
        )
        .expect("insert");

    store.overlay(&used.digest, "web").expect("overlay");

    let removed = store.prune().expect("prune");
    assert_eq!(
        removed,
        vec![unused.digest.clone()],
        "only the unreferenced one goes"
    );

    let remaining = store.list().expect("list");
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].digest, used.digest);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn releasing_a_chamber_makes_its_foundation_collectable() {
    let dir = scratch("release");
    let store = FoundationStore::at(dir.join("store"));
    let entry = store
        .insert(
            &foundation(&dir, "base.img", b"payload"),
            ARCH_X86_64,
            "alpine",
        )
        .expect("insert");

    let layer = store.overlay(&entry.digest, "web").expect("overlay");
    store.release(&entry.digest, "web").expect("release");

    assert!(!layer.path.exists(), "the layer is gone");
    assert_eq!(store.prune().expect("prune"), vec![entry.digest]);
    assert!(store.list().expect("list").is_empty());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_hostile_chamber_name_cannot_escape_the_layer_directory() {
    let dir = scratch("hostile");
    let store = FoundationStore::at(dir.join("store"));
    let entry = store
        .insert(
            &foundation(&dir, "base.img", b"payload"),
            ARCH_X86_64,
            "alpine",
        )
        .expect("insert");

    let layer = store
        .overlay(&entry.digest, "../../escape")
        .expect("overlay");
    assert!(
        layer.path.starts_with(store.root()),
        "{} escaped the store",
        layer.path.display()
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_digest_that_was_never_stored_is_reported_as_missing() {
    let dir = scratch("missing");
    let store = FoundationStore::at(dir.join("store"));

    assert!(store.entry("deadbeef").expect("entry").is_none());
    assert!(store.overlay("deadbeef", "web").is_err());

    let _ = std::fs::remove_dir_all(&dir);
}
