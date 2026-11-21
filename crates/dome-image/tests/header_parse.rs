//! Header parsing: the checks that stop a malformed file from being trusted.

use dome_image::format::{
    CHUNK_RECORD_LEN, FormatError, HASH_LEN, HEADER_PREFIX_LEN, MAGIC, NONCE_LEN, SIGNATURE_LEN,
    TAG_LEN, VERSION,
};
use dome_image::header::DomeHeader;

fn sample_header() -> DomeHeader {
    DomeHeader::new(128, [7u8; NONCE_LEN])
}

#[test]
fn header_round_trips() {
    let header = sample_header();

    let bytes = header.write();
    assert_eq!(bytes.len(), HEADER_PREFIX_LEN);

    let parsed = DomeHeader::parse(&bytes).expect("parse should succeed");
    assert_eq!(parsed, header);
}

#[test]
fn magic_is_at_the_front() {
    let bytes = sample_header().write();
    assert_eq!(&bytes[0..4], &MAGIC);
    assert_eq!(&bytes[0..4], b"DOME");
}

#[test]
fn body_length_matches_the_declared_layout() {
    let header = sample_header();
    assert_eq!(header.body_length(), HEADER_PREFIX_LEN as u64 + 128);
}

#[test]
fn wrong_magic_is_rejected() {
    let mut bytes = sample_header().write();
    bytes[0] = b'X';

    match DomeHeader::parse(&bytes) {
        Err(FormatError::BadMagic { found, expected }) => {
            assert_eq!(found[0], b'X');
            assert_eq!(expected, MAGIC);
        }
        other => panic!("expected BadMagic, got {other:?}"),
    }
}

#[test]
fn an_unknown_version_is_rejected() {
    let mut bytes = sample_header().write();
    bytes[4..8].copy_from_slice(&(VERSION + 1).to_le_bytes());

    match DomeHeader::parse(&bytes) {
        Err(FormatError::UnsupportedVersion { found, supported }) => {
            assert_eq!(found, VERSION + 1);
            assert_eq!(supported, VERSION);
        }
        other => panic!("expected UnsupportedVersion, got {other:?}"),
    }
}

#[test]
fn a_truncated_header_is_rejected() {
    let bytes = sample_header().write();

    for length in [0, 4, 8, HEADER_PREFIX_LEN - 1] {
        match DomeHeader::parse(&bytes[..length]) {
            Err(FormatError::Truncated { needed, found }) => {
                assert_eq!(needed, HEADER_PREFIX_LEN);
                assert_eq!(found, length);
            }
            other => panic!("expected Truncated at {length} bytes, got {other:?}"),
        }
    }
}

#[test]
fn a_manifest_shorter_than_a_tag_is_rejected() {
    // A ciphertext always includes a GCM tag, so 0..16 bytes cannot be a real manifest.
    let header = DomeHeader::new((TAG_LEN - 1) as u32, [0u8; NONCE_LEN]);

    match DomeHeader::parse(&header.write()) {
        Err(FormatError::ManifestTooShort { found, minimum }) => {
            assert_eq!(found, (TAG_LEN - 1) as u32);
            assert_eq!(minimum, TAG_LEN);
        }
        other => panic!("expected ManifestTooShort, got {other:?}"),
    }
}

#[test]
fn the_layout_constants_agree_with_the_documented_sizes() {
    // Header prefix: magic(4) + version(4) + manifest_length(4) + nonce(12).
    assert_eq!(HEADER_PREFIX_LEN, 4 + 4 + 4 + 1 + NONCE_LEN);
    // Chunk record: offset(8) + length(8) + nonce(12) + hash(32).
    assert_eq!(CHUNK_RECORD_LEN, 8 + 8 + NONCE_LEN + HASH_LEN);
    assert_eq!(CHUNK_RECORD_LEN, 60);
    assert_eq!(HASH_LEN, 32);
    assert_eq!(SIGNATURE_LEN, 64);
}

#[test]
fn foundation_format_round_trips_through_its_wire_byte() {
    use dome_image::FoundationFormat;

    for format in [
        FoundationFormat::DomeFoundation,
        FoundationFormat::EnginePreparedRaw,
        FoundationFormat::EnginePreparedSparse,
        FoundationFormat::EnginePreparedVhdx,
    ] {
        let byte = format.to_wire();
        assert_eq!(
            FoundationFormat::from_wire(byte).expect("round trip"),
            format,
            "format did not survive encoding as {byte}"
        );
    }
}

#[test]
fn wire_bytes_are_pinned_so_existing_files_keep_meaning() {
    use dome_image::FoundationFormat;

    // These numbers are a compatibility promise: changing them reinterprets files on disk.
    assert_eq!(FoundationFormat::DomeFoundation.to_wire(), 1);
    assert_eq!(FoundationFormat::EnginePreparedRaw.to_wire(), 2);
    assert_eq!(FoundationFormat::EnginePreparedSparse.to_wire(), 3);
    assert_eq!(FoundationFormat::EnginePreparedVhdx.to_wire(), 4);
}

#[test]
fn an_unknown_format_byte_is_a_typed_error() {
    use dome_image::FoundationFormat;
    use dome_image::format::FormatError;

    match FoundationFormat::from_wire(0xFF) {
        Err(FormatError::UnknownFoundationFormat(byte)) => assert_eq!(byte, 0xFF),
        other => panic!("expected UnknownFoundationFormat, got {other:?}"),
    }
}

#[test]
fn the_crypto_suite_is_recorded_and_classical_is_the_only_supported_one() {
    let bytes = sample_header().write();
    assert_eq!(bytes[12], dome_image::format::SUITE_CLASSICAL);

    let mut unknown = bytes.clone();
    unknown[12] = 2;
    match DomeHeader::parse(&unknown) {
        Err(FormatError::UnsupportedSuite { found, supported }) => {
            assert_eq!(found, 2);
            assert_eq!(supported, 1);
        }
        other => panic!("expected UnsupportedSuite, got {other:?}"),
    }
}
