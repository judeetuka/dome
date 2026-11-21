//! Cryptographic primitives for Dome.
//!
//! Every operation here is a standard, reviewed primitive. Nothing in this crate invents a
//! cipher, a mode, a KDF or a signature scheme: a custom file format is packaging, and
//! packaging must not replace cryptography (`.rules/security.md`).
//!
//! This crate knows nothing about the `.dome` container. It takes bytes and returns bytes.

pub mod pq;

use std::io::{Read, Write};
use std::str::FromStr;

use aws_lc_rs::aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey};
use aws_lc_rs::rand::{SecureRandom, SystemRandom};
use aws_lc_rs::signature::{ED25519, KeyPair, UnparsedPublicKey};
use thiserror::Error;

/// Re-exported so callers can hold a signing key without depending on aws-lc-rs directly.
pub use aws_lc_rs::signature::Ed25519KeyPair;

/// Length of an AES-256 key in bytes.
pub const KEY_LEN: usize = 32;
/// Length of a GCM nonce in bytes.
pub const NONCE_LEN: usize = 12;
/// Length of a GCM authentication tag in bytes.
pub const TAG_LEN: usize = 16;
/// Length of an Ed25519 signature in bytes.
pub const SIGNATURE_LEN: usize = 64;

/// Everything that can go wrong in this crate.
#[derive(Debug, Error)]
pub enum CryptoError {
    /// Decryption failed its authentication check: wrong key, or altered data.
    #[error("authentication failed: the key or the data is wrong")]
    AuthenticationFailed,

    /// The input was too short to contain the framing it claims.
    #[error("input too short: expected at least {expected} bytes, found {actual}")]
    Truncated {
        /// Bytes the operation needs.
        expected: usize,
        /// Bytes actually supplied.
        actual: usize,
    },

    /// Key material was the wrong length or otherwise unusable.
    #[error("invalid key material: {0}")]
    InvalidKey(String),

    /// Key wrapping failed.
    #[error("key wrapping failed: {0}")]
    Wrap(String),

    /// Key unwrapping failed.
    #[error("key unwrapping failed: {0}")]
    Unwrap(String),

    /// A signature did not verify.
    #[error("signature verification failed: {0}")]
    VerificationFailed(String),

    /// The platform random source refused to produce bytes.
    #[error("random source unavailable: {0}")]
    Random(String),
}

/// Generate a random 256-bit key from the platform CSPRNG.
///
/// Uses `SystemRandom` from aws-lc-rs, which reads the operating system's cryptographic
/// random source. No userspace seeding, no deterministic fallback.
pub fn generate_file_key() -> Result<[u8; KEY_LEN], CryptoError> {
    let mut key = [0u8; KEY_LEN];
    SystemRandom::new()
        .fill(&mut key)
        .map_err(|_| CryptoError::Random("the system random source failed".to_owned()))?;

    Ok(key)
}

/// Encrypt a chunk with AES-256-GCM.
///
/// Uses AES-256-GCM from aws-lc-rs for authenticated encryption. A fresh random 96-bit nonce
/// is generated per call and prepended to the output, so the layout is
/// `nonce (12) || ciphertext || tag (16)`. The nonce must never repeat under one key, which
/// is why it is generated here rather than accepted from the caller.
pub fn seal_chunk(data: &[u8], key: &[u8; KEY_LEN]) -> Result<Vec<u8>, CryptoError> {
    let less_safe = open_key(key)?;

    let mut nonce_bytes = [0u8; NONCE_LEN];
    SystemRandom::new()
        .fill(&mut nonce_bytes)
        .map_err(|_| CryptoError::Random("the system random source failed".to_owned()))?;

    // seal_in_place_append_tag extends a Vec with the tag, so the plaintext is copied in
    // first and the nonce is prepended afterwards.
    let mut ciphertext = data.to_vec();
    let nonce = Nonce::assume_unique_for_key(nonce_bytes);
    less_safe
        .seal_in_place_append_tag(nonce, Aad::empty(), &mut ciphertext)
        .map_err(|_| CryptoError::InvalidKey("sealing failed".to_owned()))?;

    let mut output = Vec::with_capacity(NONCE_LEN + ciphertext.len());
    output.extend_from_slice(&nonce_bytes);
    output.extend_from_slice(&ciphertext);

    Ok(output)
}

/// Decrypt and authenticate a chunk with AES-256-GCM.
///
/// Uses AES-256-GCM from aws-lc-rs. The nonce is read from the first 12 bytes and the
/// authentication tag must verify before any plaintext is returned: a wrong key or a modified
/// byte produces [`CryptoError::AuthenticationFailed`], never partial output.
pub fn unseal_chunk(encrypted: &[u8], key: &[u8; KEY_LEN]) -> Result<Vec<u8>, CryptoError> {
    if encrypted.len() < NONCE_LEN + TAG_LEN {
        return Err(CryptoError::Truncated {
            expected: NONCE_LEN + TAG_LEN,
            actual: encrypted.len(),
        });
    }

    let less_safe = open_key(key)?;

    let mut nonce_bytes = [0u8; NONCE_LEN];
    nonce_bytes.copy_from_slice(&encrypted[..NONCE_LEN]);
    let nonce = Nonce::assume_unique_for_key(nonce_bytes);

    let mut payload = encrypted[NONCE_LEN..].to_vec();
    let plaintext = less_safe
        .open_in_place(nonce, Aad::empty(), &mut payload)
        .map_err(|_| CryptoError::AuthenticationFailed)?;

    Ok(plaintext.to_vec())
}

/// Wrap a file key to an age recipient.
///
/// Uses the age format with an X25519 recipient (`age1...`). age is a reviewed, specified
/// format, so the wrapping is not something this crate invented: it hands the bytes to
/// `age::Encryptor` and returns what comes back.
pub fn wrap_file_key(
    key: &[u8; KEY_LEN],
    recipient: &age::x25519::Recipient,
) -> Result<Vec<u8>, CryptoError> {
    let encryptor =
        age::Encryptor::with_recipients(std::iter::once(recipient as &dyn age::Recipient))
            .map_err(|error| CryptoError::Wrap(format!("recipient rejected: {error}")))?;

    let mut wrapped = Vec::new();
    let mut writer = encryptor
        .wrap_output(&mut wrapped)
        .map_err(|error| CryptoError::Wrap(format!("cannot start the age stream: {error}")))?;

    writer
        .write_all(key)
        .map_err(|error| CryptoError::Wrap(format!("cannot write the key: {error}")))?;
    writer
        .finish()
        .map_err(|error| CryptoError::Wrap(format!("cannot finish the age stream: {error}")))?;

    Ok(wrapped)
}

/// Unwrap a file key that was wrapped to an age identity.
///
/// Uses the age format with an X25519 identity. A key wrapped to a different identity fails
/// with [`CryptoError::Unwrap`] rather than returning a wrong key.
pub fn unwrap_file_key(
    wrapped: &[u8],
    identity: &age::x25519::Identity,
) -> Result<[u8; KEY_LEN], CryptoError> {
    let decryptor = age::Decryptor::new(wrapped)
        .map_err(|error| CryptoError::Unwrap(format!("not an age file: {error}")))?;

    let mut reader = decryptor
        .decrypt(std::iter::once(identity as &dyn age::Identity))
        .map_err(|error| CryptoError::Unwrap(format!("identity rejected: {error}")))?;

    let mut key = [0u8; KEY_LEN];
    let mut filled = 0;

    while filled < KEY_LEN {
        match reader.read(&mut key[filled..]) {
            Ok(0) => break,
            Ok(count) => filled += count,
            Err(error) => return Err(CryptoError::Unwrap(format!("cannot read the key: {error}"))),
        }
    }

    if filled != KEY_LEN {
        return Err(CryptoError::Unwrap(format!(
            "wrapped key is {filled} bytes, expected {KEY_LEN}"
        )));
    }

    Ok(key)
}

/// Generate an Ed25519 signing key pair.
///
/// Uses Ed25519 from aws-lc-rs. Returns the key's PKCS#8 document and its 32-byte public key.
///
/// The document is returned rather than a raw seed because aws-lc-rs serialises the public
/// key inside it: slicing a fixed offset out of it produced a different key entirely, and the
/// signature test caught it. Parsing a library's serialisation by byte offset is a bug waiting
/// for a format tweak, so callers round-trip the document instead.
pub fn generate_signing_keypair() -> Result<(Vec<u8>, Vec<u8>), CryptoError> {
    let key_pair = Ed25519KeyPair::generate()
        .map_err(|_| CryptoError::Random("cannot generate a signing key".to_owned()))?;

    let document = key_pair
        .to_pkcs8()
        .map_err(|_| CryptoError::InvalidKey("cannot export the signing key".to_owned()))?;

    Ok((
        document.as_ref().to_vec(),
        key_pair.public_key().as_ref().to_vec(),
    ))
}

/// Build a signing key from the PKCS#8 document returned by [`generate_signing_keypair`].
///
/// Uses Ed25519 from aws-lc-rs, which validates the document rather than trusting it.
pub fn signing_key_from_pkcs8(document: &[u8]) -> Result<Ed25519KeyPair, CryptoError> {
    Ed25519KeyPair::from_pkcs8(document)
        .map_err(|error| CryptoError::InvalidKey(format!("unusable Ed25519 key: {error}")))
}

/// Parse an age identity from its `AGE-SECRET-KEY-1...` encoding.
///
/// Uses the age format's X25519 identity encoding.
/// Generate a fresh key-wrapping identity, returning its secret and public forms.
///
/// The secret is what a release's file key is wrapped to; the public form can be published,
/// which is what lets someone else wrap a key you can open.
pub fn generate_identity() -> Result<(String, String), CryptoError> {
    let identity = age::x25519::Identity::generate();
    let recipient = identity.to_public();

    // The secret is deliberately wrapped by age; exposing it is the caller's explicit act,
    // which is why this reads as a decision rather than a conversion.
    use secrecy::ExposeSecret;
    Ok((
        identity.to_string().expose_secret().to_owned(),
        recipient.to_string(),
    ))
}

/// Parse an identity from text.
pub fn identity_from_string(text: &str) -> Result<age::x25519::Identity, CryptoError> {
    age::x25519::Identity::from_str(text.trim())
        .map_err(|error| CryptoError::InvalidKey(format!("unusable age identity: {error}")))
}

/// Parse an age recipient from its `age1...` encoding.
///
/// Uses the age format's X25519 recipient encoding.
pub fn recipient_from_string(text: &str) -> Result<age::x25519::Recipient, CryptoError> {
    age::x25519::Recipient::from_str(text.trim())
        .map_err(|error| CryptoError::InvalidKey(format!("unusable age recipient: {error}")))
}

/// Sign a manifest with Ed25519.
///
/// Uses Ed25519 from aws-lc-rs. The signature is 64 bytes and covers exactly the bytes given.
pub fn sign_manifest(data: &[u8], key_pair: &Ed25519KeyPair) -> Vec<u8> {
    key_pair.sign(data).as_ref().to_vec()
}

/// Verify an Ed25519 signature over a manifest.
///
/// Uses Ed25519 from aws-lc-rs. Returns [`CryptoError::VerificationFailed`] for a bad
/// signature, a wrong key, or a malformed key: all three mean the same thing to a caller.
pub fn verify_manifest(
    data: &[u8],
    signature: &[u8],
    public_key: &[u8],
) -> Result<(), CryptoError> {
    if signature.len() != SIGNATURE_LEN {
        return Err(CryptoError::VerificationFailed(format!(
            "signature is {} bytes, expected {SIGNATURE_LEN}",
            signature.len()
        )));
    }

    UnparsedPublicKey::new(&ED25519, public_key)
        .verify(data, signature)
        .map_err(|_| CryptoError::VerificationFailed("signature does not match".to_owned()))
}

/// Build the AEAD key handle for AES-256-GCM.
fn open_key(key: &[u8; KEY_LEN]) -> Result<LessSafeKey, CryptoError> {
    let unbound = UnboundKey::new(&AES_256_GCM, key)
        .map_err(|_| CryptoError::InvalidKey("unusable AES-256 key".to_owned()))?;

    Ok(LessSafeKey::new(unbound))
}

/// Encrypt with AES-256-GCM using a caller-supplied nonce, returning ciphertext and tag.
///
/// Uses AES-256-GCM from aws-lc-rs. Unlike [seal_chunk] the nonce is not prepended: a
/// container that stores the nonce in its own structure needs the ciphertext alone. The
/// caller becomes responsible for never reusing a nonce under one key, which is why this
/// function is named the way it is.
pub fn seal_with_nonce(
    data: &[u8],
    key: &[u8; KEY_LEN],
    nonce_bytes: &[u8; NONCE_LEN],
) -> Result<Vec<u8>, CryptoError> {
    let less_safe = open_key(key)?;
    let nonce = Nonce::assume_unique_for_key(*nonce_bytes);

    let mut ciphertext = data.to_vec();
    less_safe
        .seal_in_place_append_tag(nonce, Aad::empty(), &mut ciphertext)
        .map_err(|_| CryptoError::InvalidKey("sealing failed".to_owned()))?;

    Ok(ciphertext)
}

/// Decrypt and authenticate ciphertext produced by [seal_with_nonce].
///
/// Uses AES-256-GCM from aws-lc-rs. The tag must verify before any plaintext is returned.
pub fn unseal_with_nonce(
    ciphertext: &[u8],
    key: &[u8; KEY_LEN],
    nonce_bytes: &[u8; NONCE_LEN],
) -> Result<Vec<u8>, CryptoError> {
    if ciphertext.len() < TAG_LEN {
        return Err(CryptoError::Truncated {
            expected: TAG_LEN,
            actual: ciphertext.len(),
        });
    }

    let less_safe = open_key(key)?;
    let nonce = Nonce::assume_unique_for_key(*nonce_bytes);

    let mut payload = ciphertext.to_vec();
    let plaintext = less_safe
        .open_in_place(nonce, Aad::empty(), &mut payload)
        .map_err(|_| CryptoError::AuthenticationFailed)?;

    Ok(plaintext.to_vec())
}

/// Generate a random 96-bit nonce.
///
/// Uses SystemRandom from aws-lc-rs. Callers that store nonces in their own format use this
/// rather than inventing one from a counter or a timestamp.
pub fn random_nonce() -> Result<[u8; NONCE_LEN], CryptoError> {
    let mut nonce = [0u8; NONCE_LEN];
    SystemRandom::new()
        .fill(&mut nonce)
        .map_err(|_| CryptoError::Random("the system random source failed".to_owned()))?;

    Ok(nonce)
}

/// SHA-256 digest of the input.
///
/// Uses SHA-256 from aws-lc-rs. Chunk hashes let a reader detect a swapped chunk even when
/// each chunk authenticates on its own, because authentication alone does not bind a chunk
/// to a position.
/// A streaming SHA-256, for content that is too large to hold in memory.
///
/// Foundation images run to hundreds of megabytes, so digesting one must not mean allocating
/// it. The result is identical to [`sha256`] on the same bytes.
pub struct Sha256 {
    context: aws_lc_rs::digest::Context,
}

impl Default for Sha256 {
    fn default() -> Self {
        Self::new()
    }
}

impl Sha256 {
    /// Start a digest.
    pub fn new() -> Self {
        Self {
            context: aws_lc_rs::digest::Context::new(&aws_lc_rs::digest::SHA256),
        }
    }

    /// Absorb more bytes.
    pub fn update(&mut self, bytes: &[u8]) {
        self.context.update(bytes);
    }

    /// Finish and return the digest.
    pub fn finish(self) -> [u8; 32] {
        let digest = self.context.finish();
        let mut output = [0u8; 32];
        output.copy_from_slice(digest.as_ref());
        output
    }
}

pub fn sha256(data: &[u8]) -> [u8; 32] {
    let digest = aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA256, data);
    let mut output = [0u8; 32];
    output.copy_from_slice(digest.as_ref());
    output
}
