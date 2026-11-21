//! Container round-trip and tamper tests.
//!
//! Every negative case is a real attack or a real corruption, not a synthetic API misuse:
//! flipped ciphertext, forged signature, wrong recipient, and a chunk table edited by someone
//! who does have a valid signing key.

use dome_core::{FoundationName, StorageSize};
use dome_crypto::{
    generate_file_key, generate_signing_keypair, sign_manifest, signing_key_from_pkcs8,
};
use dome_image::FoundationFormat;
use dome_image::FoundationManifest;
use dome_image::image_error::ImageError;
use dome_image::reader::read_container;
use dome_image::writer::{CHUNK_SIZE, write_container};

use dome_image::format::{CHUNK_RECORD_LEN, HEADER_PREFIX_LEN, NONCE_LEN, SIGNATURE_LEN};

struct Fixture {
    key: [u8; 32],
    signing: dome_crypto::Ed25519KeyPair,
    public: Vec<u8>,
    manifest: FoundationManifest,
}

fn fixture() -> Fixture {
    let (document, public) = generate_signing_keypair().expect("keypair");
    let signing = signing_key_from_pkcs8(&document).expect("signing key");

    let manifest = FoundationManifest::new(
        FoundationName::new("dome/alpine:3.20").expect("name"),
        FoundationFormat::DomeFoundation,
        StorageSize::from_mib(2048).expect("size"),
        "/var/lib/dome/foundations/alpine.img",
    );

    Fixture {
        key: generate_file_key().expect("key"),
        signing,
        public,
        manifest,
    }
}

fn build(fixture: &Fixture, payload: &[u8]) -> Vec<u8> {
    write_container(&fixture.manifest, payload, &fixture.key, &fixture.signing)
        .expect("write container")
}

#[test]
fn a_single_chunk_round_trips() {
    let fixture = fixture();
    let payload = b"foundation image bytes";

    let bytes = build(&fixture, payload);
    let read = read_container(&bytes, &fixture.key, &fixture.public).expect("read container");

    assert_eq!(read.payload, payload);
    assert_eq!(read.chunk_count, 1);
    assert_eq!(
        read.manifest.base_path,
        "/var/lib/dome/foundations/alpine.img"
    );
    assert_eq!(read.manifest.format, FoundationFormat::DomeFoundation);
    assert_eq!(read.manifest.virtual_size.as_mib(), 2048);
}

#[test]
fn a_payload_spanning_several_chunks_round_trips_in_order() {
    let fixture = fixture();

    // Distinct bytes per region, so a chunk swapped out of order changes the result.
    let mut payload = Vec::new();
    for chunk in 0..5u8 {
        payload.extend(std::iter::repeat_n(chunk, CHUNK_SIZE));
    }
    payload.truncate(CHUNK_SIZE * 4 + 777);

    let bytes = build(&fixture, &payload);
    let read = read_container(&bytes, &fixture.key, &fixture.public).expect("read container");

    assert_eq!(read.chunk_count, 5);
    assert_eq!(read.payload, payload);
}

#[test]
fn an_empty_payload_is_a_valid_container() {
    let fixture = fixture();

    let bytes = build(&fixture, b"");
    let read = read_container(&bytes, &fixture.key, &fixture.public).expect("read container");

    assert_eq!(read.chunk_count, 0);
    assert!(read.payload.is_empty());
}

#[test]
fn the_wrong_file_key_cannot_open_it() {
    let fixture = fixture();
    let bytes = build(&fixture, b"secret image bytes");

    let wrong = generate_file_key().expect("key");
    let error = read_container(&bytes, &wrong, &fixture.public).unwrap_err();

    assert!(matches!(error, ImageError::Crypto(_)), "got {error:?}");
}

#[test]
fn the_wrong_public_key_is_rejected_before_anything_is_decrypted() {
    let fixture = fixture();
    let bytes = build(&fixture, b"secret image bytes");

    let (_doc, other_public) = generate_signing_keypair().expect("keypair");
    let error = read_container(&bytes, &fixture.key, &other_public).unwrap_err();

    assert!(
        matches!(error, ImageError::SignatureInvalid),
        "got {error:?}"
    );
}

#[test]
fn a_flipped_ciphertext_byte_breaks_the_signature() {
    let fixture = fixture();
    let mut bytes = build(&fixture, b"some bytes that are long enough");

    // Flip a byte inside the chunk payload, which the signature covers.
    let last_payload = bytes.len() - SIGNATURE_LEN - 1;
    bytes[last_payload] ^= 0x01;

    let error = read_container(&bytes, &fixture.key, &fixture.public).unwrap_err();
    assert!(
        matches!(error, ImageError::SignatureInvalid),
        "got {error:?}"
    );
}

#[test]
fn a_forged_signature_is_rejected() {
    let fixture = fixture();
    let mut bytes = build(&fixture, b"payload");

    let last = bytes.len() - 1;
    bytes[last] ^= 0x01;

    let error = read_container(&bytes, &fixture.key, &fixture.public).unwrap_err();
    assert!(
        matches!(error, ImageError::SignatureInvalid),
        "got {error:?}"
    );
}

#[test]
fn a_chunk_moved_out_of_position_is_caught_by_its_hash() {
    let fixture = fixture();

    let mut payload = Vec::new();
    payload.extend(std::iter::repeat_n(b'A', CHUNK_SIZE));
    payload.extend(std::iter::repeat_n(b'B', CHUNK_SIZE));

    let mut bytes = build(&fixture, &payload);

    let manifest_len = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as usize;
    let records = HEADER_PREFIX_LEN + manifest_len + 4;

    // Redirect chunk 0 at chunk 1's ciphertext: take offset, length and nonce from the second
    // record but keep chunk 0's own hash. Keeping the hash is what makes this the real attack -
    // copying the whole record would move chunk 1's hash too, and the swap would look valid.
    let second = records + CHUNK_RECORD_LEN;
    let other_record = bytes[second..second + CHUNK_RECORD_LEN].to_vec();
    let head_len = 16 + NONCE_LEN;
    bytes[records..records + head_len].copy_from_slice(&other_record[..head_len]);

    // Re-sign, so the signature is valid and the hash check is the only defence left.
    let signed_len = bytes.len() - SIGNATURE_LEN;
    let signature = sign_manifest(&bytes[..signed_len], &fixture.signing);
    bytes.truncate(signed_len);
    bytes.extend_from_slice(&signature);

    let error = read_container(&bytes, &fixture.key, &fixture.public).unwrap_err();
    match error {
        ImageError::ChunkHashMismatch { index } => assert_eq!(index, 0),
        other => panic!("expected a chunk hash mismatch, got {other:?}"),
    }
}

#[test]
fn a_truncated_file_is_rejected() {
    let fixture = fixture();
    let bytes = build(&fixture, b"payload that exists");

    for keep in [0, 8, HEADER_PREFIX_LEN, bytes.len() - SIGNATURE_LEN] {
        let error = read_container(&bytes[..keep], &fixture.key, &fixture.public).unwrap_err();
        assert!(
            matches!(error, ImageError::Format(_) | ImageError::SignatureInvalid),
            "truncating to {keep} bytes gave {error:?}"
        );
    }
}

#[test]
fn the_file_framing_is_stable_across_two_identical_writes() {
    let fixture = fixture();
    let payload = b"same payload";

    let first = build(&fixture, payload);
    let second = build(&fixture, payload);

    // Same length and same manifest framing, but different bytes: nonces are fresh each time.
    assert_eq!(first.len(), second.len());
    assert_ne!(first, second);
    assert_eq!(&first[0..4], b"DOME");
}
