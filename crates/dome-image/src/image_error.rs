//! Errors for container reading and writing.

use thiserror::Error;

use crate::format::FormatError;

/// Everything that can go wrong building or reading a `.dome` file.
#[derive(Debug, Error)]
pub enum ImageError {
    /// The container framing is wrong.
    #[error("container format error: {0}")]
    Format(#[from] FormatError),

    /// The manifest could not be encoded or decoded.
    #[error("manifest error: {0}")]
    Manifest(String),

    /// Encryption or decryption failed.
    #[error("crypto error: {0}")]
    Crypto(#[from] dome_crypto::CryptoError),

    /// The signature over the container did not verify.
    #[error("container signature does not verify")]
    SignatureInvalid,

    /// A chunk's plaintext does not match the hash recorded for it.
    #[error("chunk {index} does not match its recorded hash")]
    ChunkHashMismatch {
        /// Position of the chunk in the table.
        index: usize,
    },

    /// The file was structurally impossible.
    #[error("malformed container: {0}")]
    Malformed(String),
}
