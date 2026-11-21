//! The `.dome` container layout.
//!
//! A custom container is packaging, not cryptography: every byte here is plain, fixed-width
//! framing around ciphertext produced by `dome-crypto`.
//!
//! ```text
//! Header prefix (24 bytes):
//!   [0..4]    magic            DOME
//!   [4..8]    version          u32 LE
//!   [8..12]   manifest_length  u32 LE
//!   [12..24]  manifest_nonce   [u8; 12]
//! Body:
//!   manifest ciphertext        [manifest_length]
//!   chunk_count                u32 LE
//!   chunk table                chunk_count x 60 bytes
//!   signature                  [u8; 64]
//! ```
//!
//! Every field is fixed width. There is no length-prefixed or variable-length encoding, so a
//! parser can compute every offset from these constants and a malformed file cannot make it
//! guess.

use thiserror::Error;

/// Magic bytes at the start of every `.dome` file.
pub const MAGIC: [u8; 4] = *b"DOME";

/// Container version understood by this build.
pub const VERSION: u32 = 1;

/// Suite 1: X25519 key wrapping, AES-256-GCM content, Ed25519 signature. Classical only.
pub const SUITE_CLASSICAL: u8 = 1;

/// Suite 2: X25519 + ML-KEM-768 wrapping, AES-256-GCM content, Ed25519 + ML-DSA-65 signatures.
pub const SUITE_HYBRID: u8 = 2;

/// Length of the signature footer for a suite.
///
/// The footer length is **not** a constant: suite 2 carries an ML-DSA-65 signature of 3309
/// bytes next to the 64-byte Ed25519 one. A reader that used a fixed length would frame a
/// suite-2 file from the wrong offset, so every reader derives it here.
pub fn signature_len_for_suite(suite: u8) -> Result<usize, FormatError> {
    match suite {
        SUITE_CLASSICAL => Ok(SIGNATURE_LEN),
        SUITE_HYBRID => Ok(SIGNATURE_LEN + MLDSA65_SIGNATURE_LEN),
        other => Err(FormatError::UnsupportedSuite {
            found: other,
            supported: SUITE_HYBRID,
        }),
    }
}

/// Length of an ML-DSA-65 signature (FIPS 204).
pub const MLDSA65_SIGNATURE_LEN: usize = 3309;

/// Length of the fixed header prefix, excluding the manifest ciphertext.
///
/// magic(4) + version(4) + manifest_length(4) + suite(1) + nonce(12).
pub const HEADER_PREFIX_LEN: usize = 25;

/// Length of a manifest or chunk nonce (AES-256-GCM standard).
pub const NONCE_LEN: usize = 12;

/// Length of a SHA-256 digest.
pub const HASH_LEN: usize = 32;

/// Length of an Ed25519 signature.
pub const SIGNATURE_LEN: usize = 64;

/// Length of an AES-256-GCM authentication tag.
///
/// The smallest possible manifest ciphertext is a bare tag, so a `manifest_length` smaller
/// than this cannot describe a real file.
pub const TAG_LEN: usize = 16;

/// A chunk table record: offset, length, nonce, hash.
pub const CHUNK_RECORD_LEN: usize = 8 + 8 + NONCE_LEN + HASH_LEN;

/// Length of the fixed footer that follows the chunk table.
pub const FOOTER_LEN: usize = SIGNATURE_LEN;

/// Why a `.dome` file could not be read or written.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum FormatError {
    /// The file does not begin with the expected magic bytes.
    #[error("not a .dome file: magic bytes are {found:?}, expected {expected:?}")]
    BadMagic {
        /// Bytes found at the start of the file.
        found: [u8; 4],
        /// Bytes the format requires.
        expected: [u8; 4],
    },

    /// The file is a version this build does not understand.
    #[error("unsupported .dome version {found}, this build understands {supported}")]
    UnsupportedVersion {
        /// Version found in the file.
        found: u32,
        /// Version supported by this build.
        supported: u32,
    },

    /// The input ended before the structure it declared.
    #[error("truncated .dome file: needed {needed} bytes, found {found}")]
    Truncated {
        /// Bytes required.
        needed: usize,
        /// Bytes available.
        found: usize,
    },

    /// The container names a cryptographic suite this build does not implement.
    #[error("unsupported crypto suite {found}; this build implements {supported}")]
    UnsupportedSuite {
        /// Suite recorded in the file.
        found: u8,
        /// Suite this build implements.
        supported: u8,
    },

    /// A format byte does not correspond to a known foundation format.
    #[error("unknown foundation format byte: {0}")]
    UnknownFoundationFormat(u8),

    /// A length field is too small to describe a real value.
    #[error("manifest_length {found} is below the {minimum}-byte authentication tag")]
    ManifestTooShort {
        /// Length found in the header.
        found: u32,
        /// Smallest length that could be authentic.
        minimum: usize,
    },
}

/// Wire encoding of a foundation format.
///
/// Explicit numbers rather than a cast: a reordered enum must never silently change what an
/// existing file on disk means.
impl crate::FoundationFormat {
    /// The byte this format is stored as.
    pub fn to_wire(self) -> u8 {
        match self {
            Self::DomeFoundation => 1,
            Self::EnginePreparedRaw => 2,
            Self::EnginePreparedSparse => 3,
            Self::EnginePreparedVhdx => 4,
        }
    }

    /// Read a format byte written by to_wire.
    pub fn from_wire(byte: u8) -> Result<Self, FormatError> {
        match byte {
            1 => Ok(Self::DomeFoundation),
            2 => Ok(Self::EnginePreparedRaw),
            3 => Ok(Self::EnginePreparedSparse),
            4 => Ok(Self::EnginePreparedVhdx),
            other => Err(FormatError::UnknownFoundationFormat(other)),
        }
    }
}
