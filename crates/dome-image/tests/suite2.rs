//! Suite 2 containers: framing, downgrade refusal, dual-signature enforcement.

use dome_core::{FoundationName, StorageSize};
use dome_crypto::pq::{suite2_generate_mldsa_key, suite2_verify};
use dome_crypto::{generate_file_key, generate_signing_keypair, signing_key_from_pkcs8};
use dome_image::FoundationFormat;
use dome_image::FoundationManifest;
use dome_image::format::{SUITE_HYBRID, signature_len_for_suite};
use dome_image::image_error::ImageError;
use dome_image::reader::{ContainerVerifier, read_container, read_container_with};
use dome_image::writer::{CHUNK_SIZE, write_container, write_container_suite2};

fn manifest() -> FoundationManifest {
    FoundationManifest::new(
        FoundationName::new("dome/alpine:3.20").expect("name"),
        FoundationFormat::DomeFoundation,
        StorageSize::from_mib(2048).expect("size"),
        "/tmp/base.img",
    )
}

#[test]
fn a_suite2_container_round_trips() {
    let (ed_doc, ed_public) = generate_signing_keypair().expect("ed25519");
    let ed = signing_key_from_pkcs8(&ed_doc).expect("ed25519 key");
    let (ml_key, ml_public) = suite2_generate_mldsa_key().expect("mldsa");
    let key = generate_file_key().expect("file key");

    let payload: Vec<u8> = (0..CHUNK_SIZE * 2 + 100).map(|i| (i % 251) as u8).collect();
    let bytes = write_container_suite2(&manifest(), &payload, &key, &ed, &ml_key).expect("write");

    let read = read_container_with(
        &bytes,
        &key,
        ContainerVerifier::Hybrid {
            ed25519_public: &ed_public,
            mldsa_public: &ml_public,
        },
    )
    .expect("read");

    assert_eq!(read.payload, payload);
    assert_eq!(read.chunk_count, 3);
    assert_eq!(read.suite, SUITE_HYBRID);
}

#[test]
fn the_footer_length_follows_the_suite() {
    // 64 for suite 1, 64 + 3309 for suite 2. A reader that used a constant would frame a
    // suite-2 file from the wrong offset.
    assert_eq!(signature_len_for_suite(1).expect("suite 1"), 64);
    assert_eq!(signature_len_for_suite(2).expect("suite 2"), 64 + 3309);
    assert!(signature_len_for_suite(3).is_err());
}

#[test]
fn a_suite1_reader_refuses_a_suite2_container() {
    let (ed_doc, ed_public) = generate_signing_keypair().expect("ed25519");
    let ed = signing_key_from_pkcs8(&ed_doc).expect("ed25519 key");
    let (ml_key, _ml_public) = suite2_generate_mldsa_key().expect("mldsa");
    let key = generate_file_key().expect("file key");

    let bytes = write_container_suite2(&manifest(), b"payload", &key, &ed, &ml_key).expect("write");

    // The dangerous outcome would be silently reading it as suite 1.
    let error = read_container(&bytes, &key, &ed_public).unwrap_err();
    match error {
        ImageError::Format(dome_image::format::FormatError::UnsupportedSuite { found, .. }) => {
            assert_eq!(found, SUITE_HYBRID);
        }
        other => panic!("expected UnsupportedSuite, got {other:?}"),
    }
}

#[test]
fn a_suite1_container_is_refused_by_a_suite2_reader() {
    let (ed_doc, ed_public) = generate_signing_keypair().expect("ed25519");
    let ed = signing_key_from_pkcs8(&ed_doc).expect("ed25519 key");
    let (_ml_key, ml_public) = suite2_generate_mldsa_key().expect("mldsa");
    let key = generate_file_key().expect("file key");

    let bytes = write_container(&manifest(), b"payload", &key, &ed).expect("write");

    let error = read_container_with(
        &bytes,
        &key,
        ContainerVerifier::Hybrid {
            ed25519_public: &ed_public,
            mldsa_public: &ml_public,
        },
    )
    .unwrap_err();

    assert!(
        matches!(
            error,
            ImageError::Format(dome_image::format::FormatError::UnsupportedSuite { .. })
        ),
        "got {error:?}"
    );
}

#[test]
fn a_suite2_container_is_rejected_if_the_ml_dsa_key_is_wrong() {
    let (ed_doc, ed_public) = generate_signing_keypair().expect("ed25519");
    let ed = signing_key_from_pkcs8(&ed_doc).expect("ed25519 key");
    let (ml_key, _ml_public) = suite2_generate_mldsa_key().expect("mldsa");
    let (_other_key, other_ml_public) = suite2_generate_mldsa_key().expect("mldsa");
    let key = generate_file_key().expect("file key");

    let bytes = write_container_suite2(&manifest(), b"payload", &key, &ed, &ml_key).expect("write");

    let error = read_container_with(
        &bytes,
        &key,
        ContainerVerifier::Hybrid {
            ed25519_public: &ed_public,
            mldsa_public: &other_ml_public,
        },
    )
    .unwrap_err();

    assert!(
        matches!(error, ImageError::SignatureInvalid),
        "got {error:?}"
    );
}

#[test]
fn the_suite2_signature_only_verifies_over_the_signed_bytes() {
    let (ed_doc, ed_public) = generate_signing_keypair().expect("ed25519");
    let ed = signing_key_from_pkcs8(&ed_doc).expect("ed25519 key");
    let (ml_key, ml_public) = suite2_generate_mldsa_key().expect("mldsa");
    let key = generate_file_key().expect("file key");

    let bytes = write_container_suite2(&manifest(), b"payload", &key, &ed, &ml_key).expect("write");
    let signed_len = bytes.len() - 64 - 3309;

    suite2_verify(
        &bytes[..signed_len],
        &bytes[signed_len..],
        &ed_public,
        &ml_public,
    )
    .expect("the footer should verify over everything before it");

    // Flipping a single signed byte must break it.
    let mut broken = bytes.clone();
    broken[signed_len - 1] ^= 0x01;
    assert!(
        suite2_verify(
            &broken[..signed_len],
            &bytes[signed_len..],
            &ed_public,
            &ml_public
        )
        .is_err()
    );
}
