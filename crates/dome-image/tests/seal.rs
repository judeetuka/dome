//! Seals: creation anywhere, execution only where the hardware can keep the promise.

use dome_core::{FoundationName, StorageSize};
use dome_crypto::generate_file_key;
use dome_crypto::pq::suite2_generate_mldsa_key;
use dome_crypto::{generate_signing_keypair, signing_key_from_pkcs8};
use dome_image::FoundationFormat;
use dome_image::FoundationManifest;
use dome_image::image_error::ImageError;
use dome_image::reader::ContainerVerifier;
use dome_image::seal::{
    Distribution, SECTION_CHAMBER, SECTION_CREATED, SECTION_MEMORY, SECTION_REGISTERS, Seal,
    check_resumable, distribution, read_seal, seal_is_degraded, seal_protection, write_portable,
    write_seal,
};
use dome_image::tier::{ConfidentialMechanism, HostProtection};

fn manifest() -> FoundationManifest {
    FoundationManifest::new(
        FoundationName::new("dome/alpine:3.20").expect("name"),
        FoundationFormat::DomeFoundation,
        StorageSize::from_mib(512).expect("size"),
        "/tmp/base.img",
    )
}

struct Keys {
    file: [u8; 32],
    ed: dome_crypto::Ed25519KeyPair,
    ed_public: Vec<u8>,
    ml: dome_crypto::pq::PqdsaKeyPair,
    ml_public: Vec<u8>,
}

fn keys() -> Keys {
    let (document, ed_public) = generate_signing_keypair().expect("ed25519");
    let (ml, ml_public) = suite2_generate_mldsa_key().expect("mldsa");
    Keys {
        file: generate_file_key().expect("file key"),
        ed: signing_key_from_pkcs8(&document).expect("signing key"),
        ed_public,
        ml,
        ml_public,
    }
}

fn hybrid<'a>(keys: &'a Keys) -> ContainerVerifier<'a> {
    ContainerVerifier::Hybrid {
        ed25519_public: &keys.ed_public,
        mldsa_public: &keys.ml_public,
    }
}

fn sample() -> Seal {
    let mut seal = Seal::new();
    seal.set_text(SECTION_CHAMBER, "chamber-001")
        .expect("chamber");
    seal.set_text(SECTION_CREATED, "1700000000")
        .expect("created");
    seal.set(SECTION_MEMORY, vec![0xAB; 4096]).expect("memory");
    seal.set(SECTION_REGISTERS, vec![0u8; 48])
        .expect("registers");
    seal
}

fn confidential() -> HostProtection {
    HostProtection::confidential(ConfidentialMechanism::Tdx)
}

// --- any device can create a seal ---

#[test]
fn this_device_can_create_a_seal() {
    let keys = keys();

    // Creation is not gated. This host has no TDX and still produces a seal.
    let bytes = write_seal(
        &sample(),
        &manifest(),
        HostProtection::UNPROTECTED,
        &keys.file,
        &keys.ed,
        &keys.ml,
    )
    .expect("any device can create a seal");

    let (seal, _) = read_seal(&bytes, &keys.file, hybrid(&keys)).expect("read");

    assert_eq!(
        seal_protection(&seal).expect("tier"),
        HostProtection::UNPROTECTED
    );
    // And it says so: the memory was exposed while it was captured.
    assert!(seal_is_degraded(&seal));
}

#[test]
fn a_seal_made_on_confidential_hardware_records_the_mechanism() {
    let keys = keys();
    let bytes = write_seal(
        &sample(),
        &manifest(),
        confidential(),
        &keys.file,
        &keys.ed,
        &keys.ml,
    )
    .expect("create");

    let (seal, _) = read_seal(&bytes, &keys.file, hybrid(&keys)).expect("read");

    assert_eq!(seal_protection(&seal).expect("tier"), confidential());
    assert!(!seal_is_degraded(&seal));
}

// --- but only capable hardware can run one ---

#[test]
fn this_device_cannot_run_a_seal() {
    let keys = keys();
    let bytes = write_seal(
        &sample(),
        &manifest(),
        confidential(),
        &keys.file,
        &keys.ed,
        &keys.ml,
    )
    .expect("create");
    let (seal, _) = read_seal(&bytes, &keys.file, hybrid(&keys)).expect("read");

    // The rule: creation anywhere, execution only here.
    let error = check_resumable(&seal, HostProtection::UNPROTECTED).unwrap_err();
    match error {
        ImageError::Manifest(message) => {
            assert!(message.contains("sealed release"), "{message}");
            assert!(
                message.contains("TDX"),
                "the error should name what is missing"
            );
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn a_capable_device_runs_it() {
    let keys = keys();
    let bytes = write_seal(
        &sample(),
        &manifest(),
        confidential(),
        &keys.file,
        &keys.ed,
        &keys.ml,
    )
    .expect("create");
    let (seal, _) = read_seal(&bytes, &keys.file, hybrid(&keys)).expect("read");

    let effective = check_resumable(&seal, confidential()).expect("capable hardware runs it");
    assert!(effective.is_confidential());

    let sev = HostProtection::confidential(ConfidentialMechanism::SevSnp);
    assert!(
        check_resumable(&seal, sev)
            .expect("any confidential mechanism counts")
            .is_confidential()
    );
}

#[test]
fn a_seal_created_without_protection_can_still_run_on_capable_hardware() {
    let keys = keys();
    let bytes = write_seal(
        &sample(),
        &manifest(),
        HostProtection::UNPROTECTED,
        &keys.file,
        &keys.ed,
        &keys.ml,
    )
    .expect("create");
    let (seal, _) = read_seal(&bytes, &keys.file, hybrid(&keys)).expect("read");

    // Execution depends on the device doing the running, not on where it was captured. The
    // degraded marker is what tells an operator the plaintext was exposed at capture time.
    check_resumable(&seal, confidential()).expect("capable hardware runs it");
    assert!(seal_is_degraded(&seal), "and the caveat travels with it");
}

// --- the envelope ---

#[test]
fn a_seal_round_trips_through_encryption_and_both_signatures() {
    let keys = keys();
    let seal = sample();

    let bytes = write_seal(
        &seal,
        &manifest(),
        confidential(),
        &keys.file,
        &keys.ed,
        &keys.ml,
    )
    .expect("write");
    assert_eq!(&bytes[0..4], b"DOME");

    let (recovered, container) = read_seal(&bytes, &keys.file, hybrid(&keys)).expect("read");

    for original in seal.sections() {
        assert_eq!(
            recovered.section(&original.name),
            Some(original.value.as_slice()),
            "section {:?} did not survive",
            original.name
        );
    }

    assert!(!bytes.windows(64).any(|window| window == [0xABu8; 64]));
    assert_eq!(container.manifest.virtual_size.as_mib(), 512);
}

#[test]
fn guest_memory_travels_in_the_seal() {
    let keys = keys();
    let mut seal = sample();

    let memory: Vec<u8> = (0..70_000usize).map(|index| (index % 251) as u8).collect();
    seal.set(SECTION_MEMORY, memory.clone()).expect("memory");

    let bytes = write_seal(
        &seal,
        &manifest(),
        confidential(),
        &keys.file,
        &keys.ed,
        &keys.ml,
    )
    .expect("write");
    let (recovered, _) = read_seal(&bytes, &keys.file, hybrid(&keys)).expect("read");

    assert_eq!(recovered.section(SECTION_MEMORY), Some(memory.as_slice()));
}

#[test]
fn tampering_with_a_seal_breaks_its_signature() {
    let keys = keys();
    let mut bytes = write_seal(
        &sample(),
        &manifest(),
        confidential(),
        &keys.file,
        &keys.ed,
        &keys.ml,
    )
    .expect("write");

    let last = bytes.len() - 1;
    bytes[last] ^= 0x01;

    let error = read_seal(&bytes, &keys.file, hybrid(&keys)).unwrap_err();
    assert!(
        matches!(error, ImageError::SignatureInvalid),
        "got {error:?}"
    );
}

#[test]
fn a_seal_needs_the_right_key() {
    let keys = keys();
    let bytes = write_seal(
        &sample(),
        &manifest(),
        confidential(),
        &keys.file,
        &keys.ed,
        &keys.ml,
    )
    .expect("write");

    let wrong = generate_file_key().expect("key");
    let error = read_seal(&bytes, &wrong, hybrid(&keys)).unwrap_err();
    assert!(matches!(error, ImageError::Crypto(_)), "got {error:?}");
}

// --- the other release model: portable everywhere ---

#[test]
fn a_portable_release_claims_no_confidentiality_but_runs_here() {
    let keys = keys();

    // The second model: released to everyone, protecting as much as the host can.
    let bytes = write_portable(
        &sample(),
        &manifest(),
        HostProtection::UNPROTECTED,
        &keys.file,
        &keys.ed,
        &keys.ml,
    )
    .expect("portable");

    let (release, _) = read_seal(&bytes, &keys.file, hybrid(&keys)).expect("read");

    assert_eq!(
        distribution(&release).expect("mode"),
        Distribution::Portable
    );

    // It runs on this device, and is honest that it got no confidential memory.
    let effective = check_resumable(&release, HostProtection::UNPROTECTED).expect("runs anywhere");
    assert!(
        !effective.is_confidential(),
        "a portable release on plain hardware must not claim confidentiality"
    );
}

#[test]
fn a_portable_release_gets_confidential_execution_where_it_existed() {
    let keys = keys();
    let bytes = write_portable(
        &sample(),
        &manifest(),
        HostProtection::UNPROTECTED,
        &keys.file,
        &keys.ed,
        &keys.ml,
    )
    .expect("portable");

    let (release, _) = read_seal(&bytes, &keys.file, hybrid(&keys)).expect("read");

    // The same artifact, on better hardware, runs better: that is the whole point of the
    // portable model.
    let effective = check_resumable(&release, confidential()).expect("runs");
    assert!(effective.is_confidential());
}

#[test]
fn a_sealed_release_is_distinguishable_from_a_portable_one() {
    let keys = keys();

    let sealed = write_seal(
        &sample(),
        &manifest(),
        confidential(),
        &keys.file,
        &keys.ed,
        &keys.ml,
    )
    .expect("seal");
    let portable = write_portable(
        &sample(),
        &manifest(),
        confidential(),
        &keys.file,
        &keys.ed,
        &keys.ml,
    )
    .expect("portable");

    let (sealed, _) = read_seal(&sealed, &keys.file, hybrid(&keys)).expect("read sealed");
    let (portable, _) = read_seal(&portable, &keys.file, hybrid(&keys)).expect("read portable");

    // Both were captured on confidential hardware; they differ only in where they may run.
    assert_eq!(seal_protection(&sealed).expect("tier"), confidential());
    assert_eq!(seal_protection(&portable).expect("tier"), confidential());
    assert_eq!(distribution(&sealed).expect("mode"), Distribution::Sealed);
    assert_eq!(
        distribution(&portable).expect("mode"),
        Distribution::Portable
    );

    assert!(check_resumable(&sealed, HostProtection::UNPROTECTED).is_err());
    assert!(check_resumable(&portable, HostProtection::UNPROTECTED).is_ok());
}
