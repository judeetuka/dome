//! Writing a `.dome` container.

use dome_crypto::pq::{PqdsaKeyPair, suite2_sign};
use dome_crypto::{Ed25519KeyPair, random_nonce, seal_with_nonce, sha256, sign_manifest};

use crate::FoundationManifest;
use crate::format::{
    CHUNK_RECORD_LEN, HASH_LEN, HEADER_PREFIX_LEN, NONCE_LEN, SUITE_CLASSICAL, SUITE_HYBRID,
};
use crate::header::DomeHeader;
use crate::image_error::ImageError;
use crate::manifest::encode_manifest;

/// Plaintext bytes per chunk.
pub const CHUNK_SIZE: usize = 64 * 1024;

struct SealedChunk {
    nonce: [u8; NONCE_LEN],
    hash: [u8; HASH_LEN],
    ciphertext: Vec<u8>,
}

/// Build everything except the signature footer.
fn assemble(
    manifest: &FoundationManifest,
    payload: &[u8],
    file_key: &[u8; 32],
    suite: u8,
) -> Result<Vec<u8>, ImageError> {
    let manifest_bytes = encode_manifest(
        manifest.name(),
        manifest.format(),
        manifest.virtual_size(),
        &manifest.base_path().to_string_lossy(),
    )?;

    let manifest_nonce = random_nonce()?;
    let manifest_ciphertext = seal_with_nonce(&manifest_bytes, file_key, &manifest_nonce)?;

    let mut chunks = Vec::new();
    for plaintext in payload.chunks(CHUNK_SIZE) {
        let nonce = random_nonce()?;
        chunks.push(SealedChunk {
            nonce,
            hash: sha256(plaintext),
            ciphertext: seal_with_nonce(plaintext, file_key, &nonce)?,
        });
    }

    let table_len = chunks.len() * CHUNK_RECORD_LEN;
    let payload_start = HEADER_PREFIX_LEN + manifest_ciphertext.len() + 4 + table_len;

    let mut bytes = Vec::with_capacity(payload_start + payload.len());
    bytes.extend_from_slice(
        &DomeHeader::with_suite(manifest_ciphertext.len() as u32, manifest_nonce, suite).write(),
    );
    bytes.extend_from_slice(&manifest_ciphertext);
    bytes.extend_from_slice(&(chunks.len() as u32).to_le_bytes());

    let mut offset = payload_start as u64;
    for chunk in &chunks {
        bytes.extend_from_slice(&offset.to_le_bytes());
        bytes.extend_from_slice(&(chunk.ciphertext.len() as u64).to_le_bytes());
        bytes.extend_from_slice(&chunk.nonce);
        bytes.extend_from_slice(&chunk.hash);
        offset += chunk.ciphertext.len() as u64;
    }

    for chunk in &chunks {
        bytes.extend_from_slice(&chunk.ciphertext);
    }

    Ok(bytes)
}

/// Build a suite-1 container signed with Ed25519.
pub fn write_container(
    manifest: &FoundationManifest,
    payload: &[u8],
    file_key: &[u8; 32],
    signing_key: &Ed25519KeyPair,
) -> Result<Vec<u8>, ImageError> {
    let mut bytes = assemble(manifest, payload, file_key, SUITE_CLASSICAL)?;
    let signature = sign_manifest(&bytes, signing_key);
    bytes.extend_from_slice(&signature);
    Ok(bytes)
}

/// Build a suite-2 container signed with Ed25519 **and** ML-DSA-65.
///
/// The footer is 64 + 3309 bytes rather than 64, which is why readers must derive its length
/// from the suite byte instead of a constant.
pub fn write_container_suite2(
    manifest: &FoundationManifest,
    payload: &[u8],
    file_key: &[u8; 32],
    ed25519: &Ed25519KeyPair,
    mldsa: &PqdsaKeyPair,
) -> Result<Vec<u8>, ImageError> {
    let mut bytes = assemble(manifest, payload, file_key, SUITE_HYBRID)?;
    let signature = suite2_sign(&bytes, ed25519, mldsa)?;
    bytes.extend_from_slice(&signature);
    Ok(bytes)
}
