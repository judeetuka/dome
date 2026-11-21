//! Parsing and writing the fixed `.dome` header prefix.

use crate::format::{
    FormatError, HEADER_PREFIX_LEN, MAGIC, NONCE_LEN, SUITE_CLASSICAL, TAG_LEN, VERSION,
};

/// The fixed header at the start of a `.dome` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DomeHeader {
    /// Container version.
    pub version: u32,
    /// Cryptographic suite the container uses.
    pub suite: u8,
    /// Length of the encrypted manifest that follows the header.
    pub manifest_length: u32,
    /// Nonce for the manifest ciphertext.
    pub manifest_nonce: [u8; NONCE_LEN],
}

impl DomeHeader {
    /// Build a header for the current container version.
    pub fn new(manifest_length: u32, manifest_nonce: [u8; NONCE_LEN]) -> Self {
        Self {
            version: VERSION,
            suite: crate::format::SUITE_CLASSICAL,
            manifest_length,
            manifest_nonce,
        }
    }

    /// Build a header for a specific suite.
    pub fn with_suite(manifest_length: u32, manifest_nonce: [u8; NONCE_LEN], suite: u8) -> Self {
        Self {
            version: VERSION,
            suite,
            manifest_length,
            manifest_nonce,
        }
    }

    /// Read a header from the start of `bytes`.
    ///
    /// Checks the magic, then the version, then the length. Order matters: a wrong magic is
    /// not a version problem, and a huge `manifest_length` must be refused before anything
    /// tries to read that many bytes.
    pub fn parse(bytes: &[u8]) -> Result<Self, FormatError> {
        if bytes.len() < HEADER_PREFIX_LEN {
            return Err(FormatError::Truncated {
                needed: HEADER_PREFIX_LEN,
                found: bytes.len(),
            });
        }

        let mut magic = [0u8; 4];
        magic.copy_from_slice(&bytes[0..4]);
        if magic != MAGIC {
            return Err(FormatError::BadMagic {
                found: magic,
                expected: MAGIC,
            });
        }

        let version = read_u32(bytes, 4);
        if version != VERSION {
            return Err(FormatError::UnsupportedVersion {
                found: version,
                supported: VERSION,
            });
        }

        let suite = bytes[12];
        // Both known suites are accepted; the length helpers reject anything else.
        if suite != SUITE_CLASSICAL && suite != crate::format::SUITE_HYBRID {
            return Err(FormatError::UnsupportedSuite {
                found: suite,
                supported: crate::format::SUITE_HYBRID,
            });
        }

        let manifest_length = read_u32(bytes, 8);

        // A manifest ciphertext always carries a GCM tag, so anything shorter than the tag
        // is a corrupt header rather than a small manifest. This replaced a check that could
        // never fire: with u32 lengths, 24 + length can never exceed u32::MAX + 24.
        if (manifest_length as usize) < TAG_LEN {
            return Err(FormatError::ManifestTooShort {
                found: manifest_length,
                minimum: TAG_LEN,
            });
        }

        let mut manifest_nonce = [0u8; NONCE_LEN];
        manifest_nonce.copy_from_slice(&bytes[13..HEADER_PREFIX_LEN]);

        Ok(Self {
            version,
            suite,
            manifest_length,
            manifest_nonce,
        })
    }

    /// Serialise the header prefix.
    pub fn write(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(HEADER_PREFIX_LEN);
        bytes.extend_from_slice(&MAGIC);
        bytes.extend_from_slice(&self.version.to_le_bytes());
        bytes.extend_from_slice(&self.manifest_length.to_le_bytes());
        bytes.push(self.suite);
        bytes.extend_from_slice(&self.manifest_nonce);
        bytes
    }

    /// Total length of the file implied by this header, excluding the chunk table size.
    ///
    /// Useful for validating a whole file without allocating: a caller compares this against
    /// the real length before reading anything.
    pub fn body_length(&self) -> u64 {
        HEADER_PREFIX_LEN as u64 + u64::from(self.manifest_length)
    }
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    let mut field = [0u8; 4];
    field.copy_from_slice(&bytes[offset..offset + 4]);
    u32::from_le_bytes(field)
}
