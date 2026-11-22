//! Export packages: portability, embedded seals, and the paths that must not travel.

use dome_core::{FoundationName, StorageSize};
use dome_crypto::generate_file_key;
use dome_crypto::pq::suite2_generate_mldsa_key;
use dome_crypto::{generate_signing_keypair, signing_key_from_pkcs8};
use dome_image::FoundationFormat;
use dome_image::FoundationManifest;
use dome_image::export::ExportMeta;
use dome_image::export::{
    EXPORT_FORMAT, Export, ExportMeta as Meta, PORTABLE_BASE, find_host_path, read_export,
    write_export,
};
use dome_image::image_error::ImageError;
use dome_image::reader::ContainerVerifier;
use dome_image::seal::{SECTION_CHAMBER, Seal, write_seal};
use dome_image::tier::HostProtection;

const CELL: &str = "[cell]\nfoundation = \"dome/alpine:3.20\"\nmemory_mib = 512\n";

struct Keys {
    file: [u8; 32],
    ed: dome_crypto::Ed25519KeyPair,
    ed_public: Vec<u8>,
    ml: dome_crypto::pq::PqdsaKeyPair,
    ml_public: Vec<u8>,
}

fn keys() -> Keys {
    let (document, ed_public) = generate_signing_keypair().expect("ed25519");
    let (ml, ml_public) = suite2_generate_mldsa_key().expect("mldsa");
    Keys {
        file: generate_file_key().expect("file key"),
        ed: signing_key_from_pkcs8(&document).expect("signing key"),
        ed_public,
        ml,
        ml_public,
    }
}

fn manifest(base: &str) -> FoundationManifest {
    FoundationManifest::new(
        FoundationName::new("dome/alpine:3.20").expect("name"),
        FoundationFormat::DomeFoundation,
        StorageSize::from_mib(512).expect("size"),
        base,
    )
}

fn hybrid<'a>(keys: &'a Keys) -> ContainerVerifier<'a> {
    ContainerVerifier::Hybrid {
        ed25519_public: &keys.ed_public,
        mldsa_public: &keys.ml_public,
    }
}

#[test]
fn an_export_round_trips() {
    let keys = keys();
    let export = Export::new(
        CELL,
        "dome/alpine:3.20",
        None,
        "boot completed",
        1_700_000_000,
    )
    .expect("export");

    let bytes = write_export(
        &export,
        &manifest("/tmp/base.img"),
        &keys.file,
        &keys.ed,
        &keys.ml,
    )
    .expect("write");
    assert_eq!(&bytes[0..4], b"DOME");

    let (recovered, container) = read_export(&bytes, &keys.file, hybrid(&keys)).expect("read");

    assert_eq!(recovered.format().as_deref(), Some(EXPORT_FORMAT));
    assert_eq!(recovered.cell().as_deref(), Some(CELL));
    assert_eq!(recovered.foundation().as_deref(), Some("dome/alpine:3.20"));
    assert_eq!(recovered.logs().as_deref(), Some("boot completed"));
    assert!(recovered.seal().is_none());
    assert_eq!(container.manifest.virtual_size.as_mib(), 512);
}

#[test]
fn a_seal_travels_inside_an_export_unchanged() {
    let keys = keys();

    let mut seal = Seal::new();
    seal.set_text(SECTION_CHAMBER, "chamber-001")
        .expect("chamber");
    seal.set("memory", vec![7u8; 1024]).expect("memory");

    let seal_bytes = write_seal(
        &seal,
        &manifest("/tmp/base.img"),
        HostProtection::UNPROTECTED,
        &keys.file,
        &keys.ed,
        &keys.ml,
    )
    .expect("seal");

    // The seal is encrypted, so its tier cannot be read back here; the caller declares it.
    let export = Export::with_meta(
        CELL,
        Some(seal_bytes.clone()),
        None,
        "log",
        ExportMeta::new("dome/alpine:3.20", 1).with_seal_protection(HostProtection::UNPROTECTED),
    )
    .expect("export");
    let bytes = write_export(
        &export,
        &manifest("/tmp/base.img"),
        &keys.file,
        &keys.ed,
        &keys.ml,
    )
    .expect("write");

    let (recovered, _) = read_export(&bytes, &keys.file, hybrid(&keys)).expect("read");

    // The seal is carried as an opaque container, so it can be opened separately later.
    assert_eq!(recovered.seal(), Some(seal_bytes.as_slice()));
}

#[test]
fn a_host_path_in_the_blueprint_is_refused() {
    let hostile = "[cell]\naperture = \"/home/iam/secret/data\"\n";

    match Export::new(hostile, "dome/alpine:3.20", None, "", 0) {
        Err(ImageError::Manifest(message)) => {
            assert!(message.contains("/home/iam/secret/data"), "{message}");
        }
        other => panic!("expected the host path to be refused, got {other:?}"),
    }
}

#[test]
fn a_host_path_in_the_logs_is_refused() {
    // Logs are captured text and can easily contain a path the operator should not ship.
    match Export::new(
        CELL,
        "dome/alpine:3.20",
        None,
        "error opening /root/keys/seal.pem",
        0,
    ) {
        Err(ImageError::Manifest(message)) => assert!(message.contains("/root/keys"), "{message}"),
        other => panic!("expected the host path to be refused, got {other:?}"),
    }
}

#[test]
fn the_local_foundation_path_does_not_travel() {
    let keys = keys();
    let export = Export::new(CELL, "dome/alpine:3.20", None, "", 0).expect("export");

    // The local manifest points at a home directory; the package must not.
    let bytes = write_export(
        &export,
        &manifest("/home/iam/foundations/base.img"),
        &keys.file,
        &keys.ed,
        &keys.ml,
    )
    .expect("write");

    let text = String::from_utf8_lossy(&bytes).into_owned();
    assert!(
        !text.contains("/home/iam"),
        "the export carried the local foundation path"
    );

    let (_, container) = read_export(&bytes, &keys.file, hybrid(&keys)).expect("read");
    assert_eq!(container.manifest.base_path, PORTABLE_BASE);
}

#[test]
fn an_unknown_export_format_is_refused() {
    let mut payload = Vec::new();
    let mut bundle = dome_image::bundle::Bundle::new();
    bundle
        .set_text("format", "dome-export/999")
        .expect("format");
    payload.extend_from_slice(&bundle.encode());

    match Export::decode(&payload) {
        Err(ImageError::Manifest(message)) => assert!(message.contains("999"), "{message}"),
        other => panic!("expected an unknown format error, got {other:?}"),
    }
}

#[test]
fn ordinary_container_paths_are_not_mistaken_for_host_paths() {
    // A blueprint may legitimately mention these; refusing them would make export useless.
    for benign in ["/bin/sh", "/etc/hosts", "/usr/lib/libc.so", "console=ttyS0"] {
        assert_eq!(find_host_path(benign), None, "{benign:?} was flagged");
    }

    assert_eq!(
        find_host_path("/home/iam/x").as_deref(),
        Some("/home/iam/x")
    );
    assert_eq!(
        find_host_path("prefix /root/.ssh/id").as_deref(),
        Some("/root/.ssh/id")
    );
}

#[test]
fn an_export_refuses_a_host_that_cannot_meet_the_policy() {
    // Same rule as a seal: a package that wants secrecy will not be built on a host that
    // cannot keep it.
    let error = Export::with_meta(
        CELL,
        None,
        None,
        "",
        Meta::new("dome/alpine:3.20", 0).requiring_confidentiality(),
    )
    .unwrap_err();

    assert!(format!("{error}").contains("confidential"), "{error}");
}

#[test]
fn an_export_refuses_to_understate_a_confidential_seal() {
    // The seal is encrypted, so the caller declares its tier; a mismatch is refused rather
    // than letting a package launder a confidential chamber into a plain one.
    let error = Export::with_meta(
        CELL,
        Some(vec![1, 2, 3]),
        None,
        "",
        Meta::new("dome/alpine:3.20", 0).with_seal_protection(HostProtection::confidential(
            dome_image::tier::ConfidentialMechanism::Tdx,
        )),
    )
    .unwrap_err();

    assert!(format!("{error}").contains("claims"), "{error}");
}

#[test]
fn a_packaged_seal_must_declare_its_tier() {
    let error = Export::with_meta(
        CELL,
        Some(vec![1, 2, 3]),
        None,
        "",
        Meta::new("dome/alpine:3.20", 0),
    )
    .unwrap_err();

    assert!(format!("{error}").contains("without declaring"), "{error}");
}

#[test]
fn an_export_records_its_own_tier() {
    let keys = keys();
    let export = Export::with_meta(
        CELL,
        None,
        None,
        "",
        Meta::new("dome/alpine:3.20", 7)
            .on_host(HostProtection::confidential(
                dome_image::tier::ConfidentialMechanism::SevSnp,
            ))
            .requiring_confidentiality(),
    )
    .expect("export");

    let bytes = write_export(
        &export,
        &manifest("/tmp/base.img"),
        &keys.file,
        &keys.ed,
        &keys.ml,
    )
    .expect("write");
    let (recovered, _) = read_export(&bytes, &keys.file, hybrid(&keys)).expect("read");

    assert!(recovered.protection().expect("tier").is_confidential());
    assert!(!recovered.is_degraded());
}

#[test]
fn the_key_that_opens_the_seal_travels_with_it() {
    let keys = keys();
    let export = Export::with_meta(
        CELL,
        Some(vec![1, 2, 3]),
        Some(vec![9, 9, 9]),
        "",
        Meta::new("dome/alpine:3.20", 0).with_seal_protection(HostProtection::UNPROTECTED),
    )
    .expect("export");

    let bytes = write_export(
        &export,
        &manifest("/tmp/base.img"),
        &keys.file,
        &keys.ed,
        &keys.ml,
    )
    .expect("write");
    let (recovered, _) = read_export(&bytes, &keys.file, hybrid(&keys)).expect("read");

    // Without this an export could not be imported anywhere but the machine that made it.
    assert_eq!(recovered.seal(), Some([1u8, 2, 3].as_slice()));
    assert_eq!(recovered.wrapped_key(), Some([9u8, 9, 9].as_slice()));
}

#[test]
fn a_key_without_a_seal_is_refused() {
    let error = Export::with_meta(CELL, None, Some(vec![1]), "", Meta::new("x", 0)).unwrap_err();
    assert!(error.to_string().contains("without the seal"), "{error}");
}
