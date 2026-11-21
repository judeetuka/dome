//! Reading a `.dome` container.
//!
//! Order matters and is deliberate. The fixed header prefix is parsed first because its size
//! is constant and it carries the suite byte; the footer length is then derived from that
//! suite; the signature is verified next; and only afterwards is any length or offset the
//! file declares believed.

use dome_crypto::pq::{SUITE2_SIGNATURE_LEN, suite2_verify};
use dome_crypto::{sha256, unseal_with_nonce, verify_manifest};

use crate::format::{
    CHUNK_RECORD_LEN, HASH_LEN, HEADER_PREFIX_LEN, NONCE_LEN, SUITE_HYBRID, signature_len_for_suite,
};
use crate::header::DomeHeader;
use crate::image_error::ImageError;
use crate::manifest::{DecodedManifest, decode_manifest};

/// How to verify a container's signature.
#[derive(Debug, Clone, Copy)]
pub enum ContainerVerifier<'a> {
    /// Suite 1: Ed25519 only.
    Classical {
        /// Ed25519 public key.
        ed25519_public: &'a [u8],
    },
    /// Suite 2: Ed25519 and ML-DSA-65, both required.
    Hybrid {
        /// Ed25519 public key.
        ed25519_public: &'a [u8],
        /// ML-DSA-65 public key.
        mldsa_public: &'a [u8],
    },
}

/// A container that has been opened and checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadContainer {
    /// The foundation metadata.
    pub manifest: DecodedManifest,
    /// The reassembled plaintext.
    pub payload: Vec<u8>,
    /// How many chunks the container holds.
    pub chunk_count: u32,
    /// The suite the container was written with.
    pub suite: u8,
}

/// Read a suite-1 container.
pub fn read_container(
    bytes: &[u8],
    file_key: &[u8; 32],
    public_key: &[u8],
) -> Result<ReadContainer, ImageError> {
    read_container_with(
        bytes,
        file_key,
        ContainerVerifier::Classical {
            ed25519_public: public_key,
        },
    )
}

/// Read a container, verifying it against whichever suite it declares.
pub fn read_container_with(
    bytes: &[u8],
    file_key: &[u8; 32],
    verifier: ContainerVerifier<'_>,
) -> Result<ReadContainer, ImageError> {
    if bytes.len() < HEADER_PREFIX_LEN {
        return Err(ImageError::Format(crate::format::FormatError::Truncated {
            needed: HEADER_PREFIX_LEN,
            found: bytes.len(),
        }));
    }

    // 1. The fixed prefix. Only constant offsets are read here, so nothing is trusted yet.
    let header = DomeHeader::parse(bytes)?;
    let footer_len = signature_len_for_suite(header.suite)?;

    if bytes.len() < HEADER_PREFIX_LEN + footer_len {
        return Err(ImageError::Format(crate::format::FormatError::Truncated {
            needed: HEADER_PREFIX_LEN + footer_len,
            found: bytes.len(),
        }));
    }

    // 2. Authenticate, using the suite the header declared.
    let signed_len = bytes.len() - footer_len;
    let (signed, signature) = bytes.split_at(signed_len);

    match verifier {
        ContainerVerifier::Classical { ed25519_public } => {
            if header.suite == SUITE_HYBRID {
                return Err(ImageError::Format(
                    crate::format::FormatError::UnsupportedSuite {
                        found: header.suite,
                        supported: crate::format::SUITE_CLASSICAL,
                    },
                ));
            }
            verify_manifest(signed, signature, ed25519_public)
                .map_err(|_| ImageError::SignatureInvalid)?;
        }
        ContainerVerifier::Hybrid {
            ed25519_public,
            mldsa_public,
        } => {
            if header.suite != SUITE_HYBRID {
                return Err(ImageError::Format(
                    crate::format::FormatError::UnsupportedSuite {
                        found: header.suite,
                        supported: SUITE_HYBRID,
                    },
                ));
            }
            if signature.len() != SUITE2_SIGNATURE_LEN {
                return Err(ImageError::SignatureInvalid);
            }
            suite2_verify(signed, signature, ed25519_public, mldsa_public)
                .map_err(|_| ImageError::SignatureInvalid)?;
        }
    }

    // 3. Only now may declared lengths be believed.
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

        if sha256(&plaintext) != hash {
            return Err(ImageError::ChunkHashMismatch { index });
        }

        payload.extend_from_slice(&plaintext);
    }

    Ok(ReadContainer {
        manifest,
        payload,
        chunk_count,
        suite: header.suite,
    })
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    let mut field = [0u8; 8];
    field.copy_from_slice(&bytes[offset..offset + 8]);
    u64::from_le_bytes(field)
}
