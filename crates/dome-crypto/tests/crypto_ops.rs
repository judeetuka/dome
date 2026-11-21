//! Cryptographic operations, tested for the properties that matter.
//!
//! Not round-trip smoke tests alone: each negative case asserts the specific failure mode,
//! because "decryption returned something" and "decryption refused" are very different
//! security outcomes.

use dome_crypto::{
    CryptoError, generate_file_key, generate_signing_keypair, seal_chunk, sign_manifest,
    signing_key_from_pkcs8, unseal_chunk, unwrap_file_key, verify_manifest, wrap_file_key,
};

#[test]
fn seal_unseal_round_trip() {
    let key = generate_file_key().expect("key");
    let plaintext = b"a chamber image chunk";

    let sealed = seal_chunk(plaintext, &key).expect("seal");
    let opened = unseal_chunk(&sealed, &key).expect("unseal");

    assert_eq!(opened, plaintext);
    assert!(
        !sealed
            .windows(plaintext.len())
            .any(|window| window == plaintext)
    );
}

#[test]
fn every_seal_uses_a_fresh_nonce() {
    let key = generate_file_key().expect("key");

    let first = seal_chunk(b"same input", &key).expect("seal");
    let second = seal_chunk(b"same input", &key).expect("seal");

    // Identical plaintext under one key must not produce identical ciphertext: that would
    // mean the nonce was reused, the classic catastrophic GCM failure.
    assert_ne!(first, second, "nonce was reused under one key");
    assert_eq!(
        unseal_chunk(&first, &key).expect("unseal"),
        unseal_chunk(&second, &key).expect("unseal")
    );
}

#[test]
fn wrong_key_fails_authentication() {
    let key = generate_file_key().expect("key");
    let wrong = generate_file_key().expect("key");

    let sealed = seal_chunk(b"secret", &key).expect("seal");
    let error = unseal_chunk(&sealed, &wrong).unwrap_err();

    assert!(
        matches!(error, CryptoError::AuthenticationFailed),
        "expected an authentication failure, got {error:?}"
    );
}

#[test]
fn a_flipped_bit_fails_authentication() {
    let key = generate_file_key().expect("key");
    let mut sealed = seal_chunk(b"secret payload", &key).expect("seal");

    let last = sealed.len() - 1;
    sealed[last] ^= 0x01;

    let error = unseal_chunk(&sealed, &key).unwrap_err();
    assert!(
        matches!(error, CryptoError::AuthenticationFailed),
        "got {error:?}"
    );
}

#[test]
fn truncated_input_is_a_typed_error_not_a_panic() {
    let key = generate_file_key().expect("key");

    let error = unseal_chunk(&[0u8; 8], &key).unwrap_err();
    assert!(
        matches!(error, CryptoError::Truncated { .. }),
        "got {error:?}"
    );
}

#[test]
fn key_wrap_round_trip() {
    let identity = age::x25519::Identity::generate();
    let recipient = identity.to_public();

    let key = generate_file_key().expect("key");
    let wrapped = wrap_file_key(&key, &recipient).expect("wrap");
    let unwrapped = unwrap_file_key(&wrapped, &identity).expect("unwrap");

    assert_eq!(key, unwrapped);
}

#[test]
fn wrong_identity_cannot_unwrap() {
    let identity = age::x25519::Identity::generate();
    let other = age::x25519::Identity::generate();

    let key = generate_file_key().expect("key");
    let wrapped = wrap_file_key(&key, &identity.to_public()).expect("wrap");

    let error = unwrap_file_key(&wrapped, &other).unwrap_err();
    assert!(matches!(error, CryptoError::Unwrap(_)), "got {error:?}");
}

#[test]
fn sign_and_verify_round_trip() {
    let (document, public) = generate_signing_keypair().expect("keypair");
    let signing_key = signing_key_from_pkcs8(&document).expect("signing key");

    let manifest = b"manifest bytes that must not be altered";
    let signature = sign_manifest(manifest, &signing_key);

    assert_eq!(signature.len(), 64);
    verify_manifest(manifest, &signature, &public).expect("signature should verify");
}

#[test]
fn verification_fails_for_tampered_data() {
    let (seed, public) = generate_signing_keypair().expect("keypair");
    let signing_key = signing_key_from_pkcs8(&seed).expect("signing key");

    let signature = sign_manifest(b"original", &signing_key);

    let error = verify_manifest(b"altered", &signature, &public).unwrap_err();
    assert!(
        matches!(error, CryptoError::VerificationFailed(_)),
        "got {error:?}"
    );
}

#[test]
fn verification_fails_for_the_wrong_public_key() {
    let (document, _public) = generate_signing_keypair().expect("keypair");
    let signing_key = signing_key_from_pkcs8(&document).expect("signing key");
    let (_other_document, other_public) = generate_signing_keypair().expect("keypair");

    let signature = sign_manifest(b"original", &signing_key);

    let error = verify_manifest(b"original", &signature, &other_public).unwrap_err();
    assert!(
        matches!(error, CryptoError::VerificationFailed(_)),
        "got {error:?}"
    );
}

#[test]
fn a_malformed_signature_is_rejected_without_panicking() {
    let (_document, public) = generate_signing_keypair().expect("keypair");

    let error = verify_manifest(b"data", &[0u8; 7], &public).unwrap_err();
    assert!(
        matches!(error, CryptoError::VerificationFailed(_)),
        "got {error:?}"
    );
}
