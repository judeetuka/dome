//! Seals: the payload format and the envelope around it.

use dome_core::{FoundationName, StorageSize};
use dome_crypto::generate_file_key;
use dome_crypto::pq::suite2_generate_mldsa_key;
use dome_crypto::{generate_signing_keypair, signing_key_from_pkcs8};
use dome_image::FoundationFormat;
use dome_image::FoundationManifest;
use dome_image::image_error::ImageError;
use dome_image::reader::ContainerVerifier;
use dome_image::seal::{
    SECTION_CHAMBER, SECTION_CREATED, SECTION_MEMORY, SECTION_REGISTERS, Seal, read_seal,
    write_seal,
};

fn manifest() -> FoundationManifest {
    FoundationManifest::new(
        FoundationName::new("dome/alpine:3.20").expect("name"),
        FoundationFormat::DomeFoundation,
        StorageSize::from_mib(512).expect("size"),
        "/tmp/base.img",
    )
}

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

fn sample_seal() -> Seal {
    let mut seal = Seal::new();
    seal.set_text(SECTION_CHAMBER, "chamber-001")
        .expect("chamber");
    seal.set_text(SECTION_CREATED, "1700000000")
        .expect("created");
    seal.set(SECTION_MEMORY, vec![0xAB; 4096]).expect("memory");
    seal.set(SECTION_REGISTERS, vec![0u8; 48])
        .expect("registers");
    seal
}

fn hybrid<'a>(keys: &'a Keys) -> ContainerVerifier<'a> {
    ContainerVerifier::Hybrid {
        ed25519_public: &keys.ed_public,
        mldsa_public: &keys.ml_public,
    }
}

#[test]
fn a_seal_round_trips_through_encryption_and_both_signatures() {
    let keys = keys();
    let seal = sample_seal();

    let bytes = write_seal(&seal, &manifest(), &keys.file, &keys.ed, &keys.ml).expect("write");

    // It is a DOME container.
    assert_eq!(&bytes[0..4], b"DOME");

    let (recovered, container) = read_seal(&bytes, &keys.file, hybrid(&keys)).expect("read");

    assert_eq!(recovered.sections(), seal.sections());
    assert_eq!(
        recovered.text(SECTION_CHAMBER).as_deref(),
        Some("chamber-001")
    );

    // The payload is encrypted, so guest memory must not appear in the clear.
    assert!(!bytes.windows(64).any(|window| window == [0xABu8; 64]));

    // And the container still knows which foundation it belongs to.
    assert_eq!(container.manifest.virtual_size.as_mib(), 512);
}

#[test]
fn guest_memory_travels_in_the_seal() {
    let keys = keys();
    let mut seal = sample_seal();

    // A recognisable pattern, larger than one chunk so the container splits it.
    let memory: Vec<u8> = (0..70_000usize).map(|index| (index % 251) as u8).collect();
    seal.set(SECTION_MEMORY, memory.clone()).expect("memory");

    let bytes = write_seal(&seal, &manifest(), &keys.file, &keys.ed, &keys.ml).expect("write");
    let (recovered, _) = read_seal(&bytes, &keys.file, hybrid(&keys)).expect("read");

    assert_eq!(recovered.section(SECTION_MEMORY), Some(memory.as_slice()));
}

#[test]
fn tampering_with_a_seal_breaks_its_signature() {
    let keys = keys();
    let mut bytes =
        write_seal(&sample_seal(), &manifest(), &keys.file, &keys.ed, &keys.ml).expect("write");

    let last = bytes.len() - 1;
    bytes[last] ^= 0x01;

    let error = read_seal(&bytes, &keys.file, hybrid(&keys)).unwrap_err();
    assert!(
        matches!(error, ImageError::SignatureInvalid),
        "got {error:?}"
    );
}

#[test]
fn a_seal_needs_the_right_key() {
    let keys = keys();
    let bytes =
        write_seal(&sample_seal(), &manifest(), &keys.file, &keys.ed, &keys.ml).expect("write");

    let wrong = generate_file_key().expect("key");
    let error = read_seal(&bytes, &wrong, hybrid(&keys)).unwrap_err();

    assert!(matches!(error, ImageError::Crypto(_)), "got {error:?}");
}

#[test]
fn section_names_are_validated() {
    let mut seal = Seal::new();

    let hostile = [
        "",
        "with space",
        "sl/ash",
        "../up",
        "a-name-that-is-far-too-long-to-be-reasonable-and-then-some-more-of-it",
    ];

    for name in hostile {
        assert!(
            seal.set(name, vec![1]).is_err(),
            "{name:?} should be refused as a section name"
        );
    }

    seal.set("good-name_1", vec![1]).expect("valid name");
}

#[test]
fn setting_a_section_twice_replaces_it() {
    let mut seal = Seal::new();
    seal.set(SECTION_CHAMBER, b"first".to_vec()).expect("set");
    seal.set(SECTION_CHAMBER, b"second".to_vec()).expect("set");

    assert_eq!(seal.sections().len(), 1);
    assert_eq!(seal.section(SECTION_CHAMBER), Some(b"second".as_slice()));
}

#[test]
fn an_absurd_section_count_is_refused_before_allocating() {
    // Four billion sections in a payload of eight bytes cannot be real.
    let mut payload = Vec::new();
    payload.extend_from_slice(&u32::MAX.to_le_bytes());
    payload.extend_from_slice(&[0u8; 4]);

    match Seal::decode(&payload) {
        Err(ImageError::Malformed(message)) => assert!(message.contains("sections"), "{message}"),
        other => panic!("expected Malformed, got {other:?}"),
    }
}

#[test]
fn a_truncated_payload_is_refused() {
    let encoded = sample_seal().encode();

    for keep in [0, 4, 8, encoded.len() - 1] {
        assert!(
            Seal::decode(&encoded[..keep]).is_err(),
            "truncating to {keep} bytes should be refused"
        );
    }
}
