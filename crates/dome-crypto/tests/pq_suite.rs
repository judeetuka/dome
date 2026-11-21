//! Suite 2: hybrid key wrapping and dual signatures.

use dome_crypto::pq::{
    SUITE2_SIGNATURE_LEN, SUITE2_WRAPPED_LEN, suite2_generate_identity, suite2_generate_mldsa_key,
    suite2_sign, suite2_unwrap, suite2_verify, suite2_wrap,
};
use dome_crypto::{generate_file_key, generate_signing_keypair, signing_key_from_pkcs8};

#[test]
fn a_wrapped_key_round_trips_through_the_hybrid() {
    let (identity, recipient) = suite2_generate_identity().expect("identity");
    let key = generate_file_key().expect("file key");

    let wrapped = suite2_wrap(&key, &recipient).expect("wrap");
    assert_eq!(wrapped.len(), SUITE2_WRAPPED_LEN);

    let recovered = suite2_unwrap(&wrapped, &identity).expect("unwrap");
    assert_eq!(recovered, key);
}

#[test]
fn the_wrapped_blob_does_not_contain_the_key() {
    let (_identity, recipient) = suite2_generate_identity().expect("identity");
    let key = generate_file_key().expect("file key");

    let wrapped = suite2_wrap(&key, &recipient).expect("wrap");
    assert!(!wrapped.windows(32).any(|w| w == key));
}

#[test]
fn a_different_identity_cannot_unwrap() {
    let (_identity, recipient) = suite2_generate_identity().expect("identity");
    let (other_identity, _other_recipient) = suite2_generate_identity().expect("identity");

    let key = generate_file_key().expect("file key");
    let wrapped = suite2_wrap(&key, &recipient).expect("wrap");

    assert!(suite2_unwrap(&wrapped, &other_identity).is_err());
}

#[test]
fn wrapping_the_same_key_twice_differs() {
    let (_identity, recipient) = suite2_generate_identity().expect("identity");
    let key = generate_file_key().expect("file key");

    let first = suite2_wrap(&key, &recipient).expect("wrap");
    let second = suite2_wrap(&key, &recipient).expect("wrap");

    // Fresh ML-KEM ciphertext and fresh X25519 ephemeral each time.
    assert_ne!(first, second);
}

#[test]
fn a_truncated_blob_is_a_typed_error() {
    let (identity, _recipient) = suite2_generate_identity().expect("identity");
    let error = suite2_unwrap(&[0u8; 16], &identity).unwrap_err();
    assert!(
        matches!(error, dome_crypto::CryptoError::Truncated { .. }),
        "got {error:?}"
    );
}

#[test]
fn dual_signatures_verify_only_when_both_halves_do() {
    let (ed_doc, ed_public) = generate_signing_keypair().expect("ed25519");
    let ed = signing_key_from_pkcs8(&ed_doc).expect("ed25519 key");
    let (ml_key, ml_public) = suite2_generate_mldsa_key().expect("mldsa");

    let data = b"container bytes to sign";
    let signature = suite2_sign(data, &ed, &ml_key).expect("sign");
    assert_eq!(signature.len(), SUITE2_SIGNATURE_LEN);

    suite2_verify(data, &signature, &ed_public, &ml_public).expect("both halves should verify");
}

#[test]
fn tampered_data_breaks_the_signature() {
    let (ed_doc, ed_public) = generate_signing_keypair().expect("ed25519");
    let ed = signing_key_from_pkcs8(&ed_doc).expect("ed25519 key");
    let (ml_key, ml_public) = suite2_generate_mldsa_key().expect("mldsa");

    let signature = suite2_sign(b"original", &ed, &ml_key).expect("sign");
    assert!(suite2_verify(b"altered", &signature, &ed_public, &ml_public).is_err());
}

#[test]
fn corrupting_only_the_ml_dsa_half_is_rejected() {
    let (ed_doc, ed_public) = generate_signing_keypair().expect("ed25519");
    let ed = signing_key_from_pkcs8(&ed_doc).expect("ed25519 key");
    let (ml_key, ml_public) = suite2_generate_mldsa_key().expect("mldsa");

    let data = b"payload";
    let mut signature = suite2_sign(data, &ed, &ml_key).expect("sign");

    // The Ed25519 half stays valid; only the lattice half is damaged. A design that checked
    // just one family would accept this.
    let last = signature.len() - 1;
    signature[last] ^= 0x01;

    assert!(
        suite2_verify(data, &signature, &ed_public, &ml_public).is_err(),
        "a broken ML-DSA half must fail even though Ed25519 verifies"
    );
}

#[test]
fn corrupting_only_the_ed25519_half_is_rejected() {
    let (ed_doc, ed_public) = generate_signing_keypair().expect("ed25519");
    let ed = signing_key_from_pkcs8(&ed_doc).expect("ed25519 key");
    let (ml_key, ml_public) = suite2_generate_mldsa_key().expect("mldsa");

    let data = b"payload";
    let mut signature = suite2_sign(data, &ed, &ml_key).expect("sign");
    signature[0] ^= 0x01;

    assert!(
        suite2_verify(data, &signature, &ed_public, &ml_public).is_err(),
        "a broken Ed25519 half must fail even though ML-DSA verifies"
    );
}
