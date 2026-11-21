//! Reading a `.dome` container.
//!
//! Order matters here and is deliberate: the signature is verified **before** any structure
//! is parsed. Signature-first means every length, offset and count the file declares has
//! already been authenticated, so a hostile file cannot steer the parser with unverified
//! numbers.

use dome_crypto::{sha256, unseal_with_nonce, verify_manifest};

use crate::format::{CHUNK_RECORD_LEN, HASH_LEN, HEADER_PREFIX_LEN, NONCE_LEN, SIGNATURE_LEN};
use crate::header::DomeHeader;
use crate::image_error::ImageError;
use crate::manifest::{DecodedManifest, decode_manifest};

/// A container that has been opened and checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadContainer {
    /// The foundation metadata.
    pub manifest: DecodedManifest,
    /// The reassembled plaintext.
    pub payload: Vec<u8>,
    /// How many chunks the container holds.
    pub chunk_count: u32,
}

/// Verify and open a container.
///
/// `public_key` is the Ed25519 key that signed it. Nothing is decrypted until the signature
/// verifies, and no chunk is trusted until both its tag and its recorded hash agree.
pub fn read_container(
    bytes: &[u8],
    file_key: &[u8; 32],
    public_key: &[u8],
) -> Result<ReadContainer, ImageError> {
    // 1. Authenticate first.
    if bytes.len() < HEADER_PREFIX_LEN + SIGNATURE_LEN {
        return Err(ImageError::Format(crate::format::FormatError::Truncated {
            needed: HEADER_PREFIX_LEN + SIGNATURE_LEN,
            found: bytes.len(),
        }));
    }

    let signed_len = bytes.len() - SIGNATURE_LEN;
    let (signed, signature) = bytes.split_at(signed_len);
    verify_manifest(signed, signature, public_key).map_err(|_| ImageError::SignatureInvalid)?;

    // 2. Only now may the structure be believed.
    let header = DomeHeader::parse(bytes)?;
    let manifest_start = HEADER_PREFIX_LEN;
    let manifest_end = manifest_start + header.manifest_length as usize;

    if manifest_end > signed_len {
        return Err(ImageError::Malformed(format!(
            "manifest ends at {manifest_end}, past the signed region of {signed_len}"
        )));
    }

    let manifest_bytes = unseal_with_nonce(
        &bytes[manifest_start..manifest_end],
        file_key,
        &header.manifest_nonce,
    )?;
    let manifest = decode_manifest(&manifest_bytes)?;

    // 3. Chunk table.
    let count_start = manifest_end;
    let count_end = count_start + 4;
    if count_end > signed_len {
        return Err(ImageError::Malformed("chunk count is missing".to_owned()));
    }

    let mut count_bytes = [0u8; 4];
    count_bytes.copy_from_slice(&bytes[count_start..count_end]);
    let chunk_count = u32::from_le_bytes(count_bytes);

    let table_start = count_end;
    let table_end = table_start
        .checked_add(chunk_count as usize * CHUNK_RECORD_LEN)
        .ok_or_else(|| ImageError::Malformed("chunk table overflows".to_owned()))?;

    if table_end > signed_len {
        return Err(ImageError::Malformed(format!(
            "chunk table needs {chunk_count} records, file is too short"
        )));
    }

    let mut payload = Vec::new();

    for index in 0..chunk_count as usize {
        let record = table_start + index * CHUNK_RECORD_LEN;

        let offset = read_u64(bytes, record);
        let length = read_u64(bytes, record + 8);

        let mut nonce = [0u8; NONCE_LEN];
        nonce.copy_from_slice(&bytes[record + 16..record + 16 + NONCE_LEN]);

        let mut hash = [0u8; HASH_LEN];
        hash.copy_from_slice(&bytes[record + 16 + NONCE_LEN..record + CHUNK_RECORD_LEN]);

        let start = usize::try_from(offset)
            .map_err(|_| ImageError::Malformed("chunk offset does not fit".to_owned()))?;
        let end = start
            .checked_add(
                usize::try_from(length)
                    .map_err(|_| ImageError::Malformed("chunk length does not fit".to_owned()))?,
            )
            .ok_or_else(|| ImageError::Malformed("chunk range overflows".to_owned()))?;

        if end > signed_len {
            return Err(ImageError::Malformed(format!(
                "chunk {index} ends at {end}, past the signed region of {signed_len}"
            )));
        }

        let plaintext = unseal_with_nonce(&bytes[start..end], file_key, &nonce)?;

        // The tag proves the chunk is authentic; the hash proves it belongs at this index.
        if sha256(&plaintext) != hash {
            return Err(ImageError::ChunkHashMismatch { index });
        }

        payload.extend_from_slice(&plaintext);
    }

    Ok(ReadContainer {
        manifest,
        payload,
        chunk_count,
    })
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    let mut field = [0u8; 8];
    field.copy_from_slice(&bytes[offset..offset + 8]);
    u64::from_le_bytes(field)
}
