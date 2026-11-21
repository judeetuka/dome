//! Chamber seals: guest state as one authenticated, encrypted artifact.
//!
//! A seal is a `.dome` container whose payload is a bundle of named sections. Metadata
//! travels inside the same envelope as the binary state, so the chamber id, the foundation
//! and the captured memory are all covered by one signature, and nothing about the seal is
//! stored outside it. That is what makes it portable to another host.

use crate::FoundationManifest;
use crate::bundle::Bundle;
use crate::image_error::ImageError;
use crate::reader::{ContainerVerifier, ReadContainer, read_container_with};
use crate::tier::HostProtection;
use crate::writer::write_container_suite2;

/// How an artifact is meant to be distributed, which decides where it may run.
///
/// Two models, both legitimate:
///
/// - A **sealed** release protects the publisher's IP. The image only ever runs where the host
///   cannot read guest memory, so a user cannot extract what is inside it. That is a hard
///   requirement: a device without TDX, SEV-SNP or a shielded VM cannot run it.
/// - A **portable** release is for everyone. It runs anywhere, and the host it lands on decides
///   how much protection it gets: confidential execution where the hardware offers it, the cage
///   and minimal device model everywhere else.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Distribution {
    /// Closed release: runs only where guest memory can be kept confidential.
    Sealed,
    /// Open release: runs anywhere, protecting as much as the host can.
    Portable,
}

impl Distribution {
    /// Stable name for the on-disk record.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sealed => "sealed",
            Self::Portable => "portable",
        }
    }

    /// Parse the on-disk form.
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "sealed" => Self::Sealed,
            "portable" => Self::Portable,
            _ => return None,
        })
    }

    /// Whether running this artifact demands confidential hardware.
    pub fn requires_confidential_hardware(self) -> bool {
        self == Self::Sealed
    }
}

/// A seal's payload.
pub type Seal = Bundle;

/// Section holding the chamber identifier.
pub const SECTION_CHAMBER: &str = "chamber";
/// Section holding the creation time, in seconds since the Unix epoch.
pub const SECTION_CREATED: &str = "created-unix";
/// Section holding the foundation name the chamber was built from.
pub const SECTION_FOUNDATION: &str = "foundation";
/// Section holding the resource limits, as text.
pub const SECTION_LIMITS: &str = "limits";
/// Section holding the guest's physical memory.
pub const SECTION_MEMORY: &str = "memory";
/// Section holding the vCPU's architectural state.
pub const SECTION_REGISTERS: &str = "registers";
/// Section recording how the artifact is meant to be distributed.
pub const SECTION_DISTRIBUTION: &str = "distribution";
/// Section recording the protection tier of the host that produced the seal.
pub const SECTION_PROTECTION: &str = "protection";
/// Section marking a seal that was produced knowingly below its desired tier.
pub const SECTION_DEGRADED: &str = "degraded";

/// Write a **sealed** release: one that runs only where guest memory can be kept confidential.
///
/// Any device may create one. What it cannot do is run it, and that is the point: a sealed
/// release is how a publisher protects their IP, so it must not execute anywhere the image
/// could be read out of guest memory.
pub fn write_seal(
    seal: &Seal,
    manifest: &FoundationManifest,
    captured_on: HostProtection,
    file_key: &[u8; 32],
    ed25519: &dome_crypto::Ed25519KeyPair,
    mldsa: &dome_crypto::pq::PqdsaKeyPair,
) -> Result<Vec<u8>, ImageError> {
    assemble(
        seal,
        manifest,
        captured_on,
        Distribution::Sealed,
        file_key,
        ed25519,
        mldsa,
    )
}

/// Write a **portable** release: one that runs anywhere, protecting as much as the host can.
///
/// Used to distribute widely. Where the hardware offers TDX, SEV-SNP or a shielded VM the guest
/// runs with confidential memory; anywhere else it runs under the cage and the minimal device
/// model, and the runtime reports which it got rather than implying more.
pub fn write_portable(
    seal: &Seal,
    manifest: &FoundationManifest,
    captured_on: HostProtection,
    file_key: &[u8; 32],
    ed25519: &dome_crypto::Ed25519KeyPair,
    mldsa: &dome_crypto::pq::PqdsaKeyPair,
) -> Result<Vec<u8>, ImageError> {
    assemble(
        seal,
        manifest,
        captured_on,
        Distribution::Portable,
        file_key,
        ed25519,
        mldsa,
    )
}

fn assemble(
    seal: &Seal,
    manifest: &FoundationManifest,
    captured_on: HostProtection,
    distribution: Distribution,
    file_key: &[u8; 32],
    ed25519: &dome_crypto::Ed25519KeyPair,
    mldsa: &dome_crypto::pq::PqdsaKeyPair,
) -> Result<Vec<u8>, ImageError> {
    let mut payload = seal.clone();
    payload.set_text(SECTION_DISTRIBUTION, distribution.as_str())?;
    payload.set_text(SECTION_PROTECTION, &captured_on.to_record())?;

    // Provenance, not a verdict: if the device that captured this could not protect the guest,
    // an auditor should see that the plaintext was exposed at capture time.
    if !captured_on.is_confidential() {
        payload.set_text(SECTION_DEGRADED, "true")?;
    }

    write_container_suite2(manifest, &payload.encode(), file_key, ed25519, mldsa)
}

/// Read how an artifact is meant to be distributed.
pub fn distribution(seal: &Seal) -> Result<Distribution, ImageError> {
    let value = seal
        .text(SECTION_DISTRIBUTION)
        .ok_or_else(|| ImageError::Manifest("artifact records no distribution".to_owned()))?;

    Distribution::parse(&value)
        .ok_or_else(|| ImageError::Manifest(format!("unknown distribution {value:?}")))
}
/// Whether a seal was captured on a device that could not protect guest memory.
pub fn seal_is_degraded(seal: &Seal) -> bool {
    seal.text(SECTION_DEGRADED).as_deref() == Some("true")
}

/// Read the protection tier a seal records.
pub fn seal_protection(seal: &Seal) -> Result<HostProtection, ImageError> {
    let value = seal
        .text(SECTION_PROTECTION)
        .ok_or_else(|| ImageError::Manifest("seal records no protection tier".to_owned()))?;

    HostProtection::parse_record(&value)
        .ok_or_else(|| ImageError::Manifest(format!("unknown protection tier {value:?}")))
}

/// Read a seal back, verifying and decrypting it first.
pub fn read_seal(
    bytes: &[u8],
    file_key: &[u8; 32],
    verifier: ContainerVerifier<'_>,
) -> Result<(Seal, ReadContainer), ImageError> {
    let container = read_container_with(bytes, file_key, verifier)?;
    let seal = Seal::decode(&container.payload)?;
    Ok((seal, container))
}

/// What protection a running artifact actually got.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct EffectiveProtection {
    /// The tier the running host provides.
    pub tier: HostProtection,
}

impl EffectiveProtection {
    /// Whether guest memory is confidential right now.
    pub fn is_confidential(self) -> bool {
        self.tier.is_confidential()
    }
}

/// Check that this device may run an artifact, and report what protection it will get.
///
/// A **sealed** release requires confidential hardware; there is no override, because the
/// whole purpose is that the image cannot be read out of guest memory. A **portable** release
/// runs anywhere, and the returned tier says what this host actually provides, so a caller can
/// report it instead of implying more than it has.
pub fn check_resumable(
    seal: &Seal,
    host: HostProtection,
) -> Result<EffectiveProtection, ImageError> {
    let mode = distribution(seal)?;

    if mode.requires_confidential_hardware() && !host.is_confidential() {
        return Err(ImageError::Manifest(format!(
            "this release is sealed and this device is {}: a sealed release runs only on hardware that keeps guest memory confidential (TDX, SEV-SNP, or a shielded VM)",
            host.to_record()
        )));
    }

    Ok(EffectiveProtection { tier: host })
}
