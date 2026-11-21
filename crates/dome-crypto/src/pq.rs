//! Suite 2: post-quantum hybrid key wrapping and dual signatures.
//!
//! Construction, deliberately not invented:
//! - KEM: X25519 together with ML-KEM-768 (FIPS 203), combined the way RFC 10024
//!   prescribes. The shared secret is the concatenation with the **ML-KEM secret first**
//!   (RFC 10024 section 4: the ML-KEM secret is placed first in X25519MLKEM768). Both
//!   secrets go through HKDF-SHA256, so the wrapped key survives unless X25519 and ML-KEM
//!   both break.
//! - Signatures: Ed25519 **and** ML-DSA-65 (FIPS 204), verified independently; both must
//!   pass. Forgery then needs curves and lattices broken together.
//!
//! What is ours rather than the RFC's: the wire framing and the HKDF context string, because
//! RFC 10024 specifies a TLS 1.3 handshake and this is an at-rest container. The combination
//! order and the primitive choices are the RFC's. Recorded in ADR 0003.

use aws_lc_rs::agreement::{self, EphemeralPrivateKey, UnparsedPublicKey, X25519};
use aws_lc_rs::hkdf::{HKDF_SHA256, KeyType, Salt};
use aws_lc_rs::kem::{Ciphertext, DecapsulationKey, EncapsulationKey, ML_KEM_768};
use aws_lc_rs::rand::{SecureRandom, SystemRandom};
use aws_lc_rs::signature::{
    KeyPair as _, ML_DSA_65, ML_DSA_65_SIGNING, UnparsedPublicKey as SigPublicKey,
};

/// Re-exported so callers can hold an ML-DSA signing key without depending on aws-lc-rs.
pub use aws_lc_rs::signature::PqdsaKeyPair;

use crate::{CryptoError, KEY_LEN, SIGNATURE_LEN, TAG_LEN};

/// ML-KEM-768 ciphertext length (FIPS 203).
pub const MLKEM768_CIPHERTEXT_LEN: usize = 1088;
/// ML-KEM-768 encapsulation (public) key length.
pub const MLKEM768_ENCAPSULATION_KEY_LEN: usize = 1184;
/// X25519 public key length.
pub const X25519_PUBLIC_LEN: usize = 32;
/// ML-DSA-65 signature length (FIPS 204).
pub const MLDSA65_SIGNATURE_LEN: usize = 3309;
/// ML-DSA-65 public key length.
pub const MLDSA65_PUBLIC_KEY_LEN: usize = 1952;

/// Length of the suite-2 wrapping blob: ML-KEM ciphertext then X25519 ephemeral public key.
pub const SUITE2_BLOB_LEN: usize = MLKEM768_CIPHERTEXT_LEN + X25519_PUBLIC_LEN;
/// Length of the suite-2 wrapping payload: blob, nonce, then the sealed file key and tag.
pub const SUITE2_WRAPPED_LEN: usize = SUITE2_BLOB_LEN + crate::NONCE_LEN + KEY_LEN + TAG_LEN;
/// Length of a suite-2 signature: Ed25519 then ML-DSA-65.
pub const SUITE2_SIGNATURE_LEN: usize = SIGNATURE_LEN + MLDSA65_SIGNATURE_LEN;

/// HKDF context string for the suite-2 key schedule.
const KDF_INFO: &[u8] = b"dome suite2 file key";

/// Everything a sender needs to wrap a file key for a recipient.
#[derive(Debug, Clone)]
pub struct Suite2Recipient {
    /// ML-KEM-768 encapsulation key, DER-encoded as aws-lc-rs exports it.
    pub mlkem_encapsulation_key: Vec<u8>,
    /// X25519 public key.
    pub x25519_public: [u8; X25519_PUBLIC_LEN],
}

/// Everything a recipient needs to unwrap.
#[derive(Debug, Clone)]
pub struct Suite2Identity {
    /// ML-KEM-768 decapsulation key.
    pub mlkem_decapsulation_key: Vec<u8>,
    /// X25519 private key, in the format aws-lc-rs accepts.
    pub x25519_private: Vec<u8>,
}

/// Generate a suite-2 identity and the recipient view of it.
pub fn suite2_generate_identity() -> Result<(Suite2Identity, Suite2Recipient), CryptoError> {
    let decapsulation = DecapsulationKey::generate(&ML_KEM_768)
        .map_err(|_| CryptoError::Random("cannot generate an ML-KEM key".to_owned()))?;

    let mlkem_dk = decapsulation
        .key_bytes()
        .map_err(|_| CryptoError::InvalidKey("cannot export the ML-KEM key".to_owned()))?
        .as_ref()
        .to_vec();

    let encapsulation = decapsulation
        .encapsulation_key()
        .map_err(|_| CryptoError::InvalidKey("cannot derive the ML-KEM public key".to_owned()))?;

    let mlkem_ek = encapsulation
        .key_bytes()
        .map_err(|_| CryptoError::InvalidKey("cannot export the ML-KEM public key".to_owned()))?
        .as_ref()
        .to_vec();

    // The private key is never exported: the identity stores the seed and rebuilds the key
    // from it, because aws-lc-rs deliberately offers no way to read a private key back out.
    let mut seed = [0u8; X25519_PUBLIC_LEN];
    SystemRandom::new()
        .fill(&mut seed)
        .map_err(|_| CryptoError::Random("cannot generate an X25519 key".to_owned()))?;

    let x_private = agreement::PrivateKey::from_private_key(&X25519, &seed)
        .map_err(|_| CryptoError::InvalidKey("unusable X25519 seed".to_owned()))?;

    let x_public = x_private
        .compute_public_key()
        .map_err(|_| CryptoError::InvalidKey("cannot derive the X25519 public key".to_owned()))?;

    let mut x25519_public = [0u8; X25519_PUBLIC_LEN];
    x25519_public.copy_from_slice(x_public.as_ref());

    Ok((
        Suite2Identity {
            mlkem_decapsulation_key: mlkem_dk,
            x25519_private: seed.to_vec(),
        },
        Suite2Recipient {
            mlkem_encapsulation_key: mlkem_ek,
            x25519_public,
        },
    ))
}

/// Derive the wrapping key from both shared secrets, ML-KEM first per RFC 10024.
fn suite2_kem_combine(
    mlkem_secret: &[u8],
    x25519_secret: &[u8],
) -> Result<[u8; KEY_LEN], CryptoError> {
    let mut combined = Vec::with_capacity(mlkem_secret.len() + x25519_secret.len());
    combined.extend_from_slice(mlkem_secret);
    combined.extend_from_slice(x25519_secret);

    let salt = Salt::new(HKDF_SHA256, &[]);
    let prk = salt.extract(&combined);

    let okm = prk
        .expand(&[KDF_INFO], Suite2KeyLen)
        .map_err(|_| CryptoError::InvalidKey("HKDF expand failed".to_owned()))?;

    let mut key = [0u8; KEY_LEN];
    okm.fill(&mut key)
        .map_err(|_| CryptoError::InvalidKey("HKDF fill failed".to_owned()))?;

    Ok(key)
}

struct Suite2KeyLen;

impl KeyType for Suite2KeyLen {
    fn len(&self) -> usize {
        KEY_LEN
    }
}

/// Wrap a file key to a suite-2 recipient.
///
/// Uses ML-KEM-768 (FIPS 203) and X25519 together, combined per RFC 10024, then
/// AES-256-GCM to seal the file key under the derived wrapping key.
pub fn suite2_wrap(
    key: &[u8; KEY_LEN],
    recipient: &Suite2Recipient,
) -> Result<Vec<u8>, CryptoError> {
    let encapsulation = EncapsulationKey::new(&ML_KEM_768, &recipient.mlkem_encapsulation_key)
        .map_err(|error| CryptoError::InvalidKey(format!("bad ML-KEM public key: {error}")))?;

    let (ciphertext, mlkem_secret) = encapsulation
        .encapsulate()
        .map_err(|_| CryptoError::InvalidKey("encapsulation failed".to_owned()))?;

    let rng = SystemRandom::new();
    let ephemeral = EphemeralPrivateKey::generate(&X25519, &rng)
        .map_err(|_| CryptoError::Random("cannot generate an X25519 key".to_owned()))?;

    let ephemeral_public = ephemeral
        .compute_public_key()
        .map_err(|_| CryptoError::InvalidKey("cannot derive the X25519 public key".to_owned()))?;

    let peer = UnparsedPublicKey::new(&X25519, recipient.x25519_public);

    let x25519_secret = agreement::agree_ephemeral(
        ephemeral,
        peer,
        CryptoError::InvalidKey("X25519 agreement failed".to_owned()),
        |secret| Ok::<Vec<u8>, CryptoError>(secret.to_vec()),
    )?;

    let wrapping_key = suite2_kem_combine(mlkem_secret.as_ref(), &x25519_secret)?;

    let nonce = crate::random_nonce()?;
    let sealed = crate::seal_with_nonce(key, &wrapping_key, &nonce)?;

    let mut blob = Vec::with_capacity(SUITE2_WRAPPED_LEN);
    blob.extend_from_slice(ciphertext.as_ref());
    blob.extend_from_slice(ephemeral_public.as_ref());
    blob.extend_from_slice(&nonce);
    blob.extend_from_slice(&sealed);

    Ok(blob)
}

/// Unwrap a file key with a suite-2 identity.
pub fn suite2_unwrap(blob: &[u8], identity: &Suite2Identity) -> Result<[u8; KEY_LEN], CryptoError> {
    if blob.len() != SUITE2_WRAPPED_LEN {
        return Err(CryptoError::Truncated {
            expected: SUITE2_WRAPPED_LEN,
            actual: blob.len(),
        });
    }

    let mut ciphertext_bytes = [0u8; MLKEM768_CIPHERTEXT_LEN];
    ciphertext_bytes.copy_from_slice(&blob[..MLKEM768_CIPHERTEXT_LEN]);

    let mut ephemeral_public = [0u8; X25519_PUBLIC_LEN];
    ephemeral_public.copy_from_slice(&blob[MLKEM768_CIPHERTEXT_LEN..SUITE2_BLOB_LEN]);

    let mut nonce = [0u8; crate::NONCE_LEN];
    nonce.copy_from_slice(&blob[SUITE2_BLOB_LEN..SUITE2_BLOB_LEN + crate::NONCE_LEN]);

    let decapsulation = DecapsulationKey::new(&ML_KEM_768, &identity.mlkem_decapsulation_key)
        .map_err(|error| CryptoError::InvalidKey(format!("bad ML-KEM private key: {error}")))?;

    let mlkem_secret = decapsulation
        .decapsulate(Ciphertext::from(&ciphertext_bytes[..]))
        .map_err(|_| CryptoError::Unwrap("decapsulation failed".to_owned()))?;

    let x_private = agreement::PrivateKey::from_private_key(&X25519, &identity.x25519_private)
        .map_err(|error| CryptoError::InvalidKey(format!("bad X25519 private key: {error}")))?;

    let peer = UnparsedPublicKey::new(&X25519, ephemeral_public);
    let x25519_secret = agreement::agree(
        &x_private,
        peer,
        CryptoError::Unwrap("X25519 agreement failed".to_owned()),
        |secret| Ok::<Vec<u8>, CryptoError>(secret.to_vec()),
    )?;

    let wrapping_key = suite2_kem_combine(mlkem_secret.as_ref(), x25519_secret.as_ref())?;

    let sealed = &blob[SUITE2_BLOB_LEN + crate::NONCE_LEN..];
    let plaintext = crate::unseal_with_nonce(sealed, &wrapping_key, &nonce)?;

    if plaintext.len() != KEY_LEN {
        return Err(CryptoError::Unwrap(
            "unwrapped key has the wrong length".to_owned(),
        ));
    }

    let mut key = [0u8; KEY_LEN];
    key.copy_from_slice(&plaintext);

    Ok(key)
}

/// Generate the ML-DSA-65 half of a suite-2 signing identity.
///
/// Uses ML-DSA-65 from aws-lc-rs (FIPS 204). Returns the key pair and its public key in the
/// X.509 DER form that aws-lc-rs verifies against, so the caller never has to parse DER.
pub fn suite2_generate_mldsa_key() -> Result<(PqdsaKeyPair, Vec<u8>), CryptoError> {
    let key_pair = PqdsaKeyPair::generate(&ML_DSA_65_SIGNING)
        .map_err(|_| CryptoError::Random("cannot generate an ML-DSA key".to_owned()))?;

    let public = key_pair.public_key().as_ref().to_vec();

    Ok((key_pair, public))
}

/// Sign with Ed25519 **and** ML-DSA-65, the Ed25519 signature first.
///
/// Both signatures cover the same bytes. Verification requires both to pass, so an attacker
/// must break curves and lattices together to forge a container.
pub fn suite2_sign(
    data: &[u8],
    ed25519: &crate::Ed25519KeyPair,
    mldsa: &PqdsaKeyPair,
) -> Result<Vec<u8>, CryptoError> {
    let ed_signature = crate::sign_manifest(data, ed25519);

    let mut ml_signature = vec![0u8; MLDSA65_SIGNATURE_LEN];
    let written = mldsa
        .sign(data, &mut ml_signature)
        .map_err(|_| CryptoError::InvalidKey("ML-DSA signing failed".to_owned()))?;
    ml_signature.truncate(written);

    let mut output = Vec::with_capacity(SIGNATURE_LEN + written);
    output.extend_from_slice(&ed_signature);
    output.extend_from_slice(&ml_signature);

    Ok(output)
}

/// Verify a suite-2 dual signature. Both halves must verify.
pub fn suite2_verify(
    data: &[u8],
    signature: &[u8],
    ed25519_public: &[u8],
    mldsa_public: &[u8],
) -> Result<(), CryptoError> {
    if signature.len() <= SIGNATURE_LEN {
        return Err(CryptoError::VerificationFailed(
            "suite-2 signature is too short to hold both halves".to_owned(),
        ));
    }

    let (ed_signature, ml_signature) = signature.split_at(SIGNATURE_LEN);

    // Checked first because it is cheap; the lattice check is the expensive one.
    crate::verify_manifest(data, ed_signature, ed25519_public)?;

    SigPublicKey::new(&ML_DSA_65, mldsa_public)
        .verify(data, ml_signature)
        .map_err(|_| {
            CryptoError::VerificationFailed("ML-DSA signature does not match".to_owned())
        })?;

    Ok(())
}

/// Serialise an ML-DSA key pair as PKCS#8, for storage.
///
/// A release has to be verifiable by whoever holds the public half, so the signing key must
/// survive a restart. PKCS#8 is the format aws-lc-rs already produces and parses, which keeps
/// the private key in a shape a standard tool can inspect rather than an ad-hoc encoding.
pub fn suite2_mldsa_to_pkcs8(key: &PqdsaKeyPair) -> Result<Vec<u8>, CryptoError> {
    // v1 is the seed form, which is what a later build can reconstruct the key from.
    let document = key.to_pkcs8v1().map_err(|error| {
        CryptoError::InvalidKey(format!("cannot serialise ML-DSA key: {error}"))
    })?;

    Ok(document.as_ref().to_vec())
}

/// Load an ML-DSA key pair from PKCS#8.
pub fn suite2_mldsa_from_pkcs8(document: &[u8]) -> Result<PqdsaKeyPair, CryptoError> {
    PqdsaKeyPair::from_pkcs8(&ML_DSA_65_SIGNING, document)
        .map_err(|error| CryptoError::InvalidKey(format!("cannot load ML-DSA key: {error}")))
}
