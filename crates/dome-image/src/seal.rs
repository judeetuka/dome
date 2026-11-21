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
use crate::writer::write_container_suite2;

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

/// Write a seal as a suite-2 container: encrypted, and signed by both key families.
pub fn write_seal(
    seal: &Seal,
    manifest: &FoundationManifest,
    file_key: &[u8; 32],
    ed25519: &dome_crypto::Ed25519KeyPair,
    mldsa: &dome_crypto::pq::PqdsaKeyPair,
) -> Result<Vec<u8>, ImageError> {
    write_container_suite2(manifest, &seal.encode(), file_key, ed25519, mldsa)
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
