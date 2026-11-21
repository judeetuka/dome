//! Chamber seals: guest state as one authenticated, encrypted artifact.
//!
//! A seal is a `.dome` container whose payload is a set of named sections. Metadata travels
//! inside the same envelope as the binary state, so the chamber id, the foundation and the
//! captured memory are all covered by one signature. Nothing about the seal is stored
//! outside it, which is what makes it portable to another host.
//!
//! Sections rather than a struct: adding a field later must not change what an old seal
//! means, and a reader that does not recognise a section can still open the rest.

use crate::FoundationManifest;
use crate::image_error::ImageError;
use crate::reader::{ContainerVerifier, ReadContainer, read_container_with};
use crate::writer::write_container_suite2;

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

/// A named piece of a seal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    /// Section name.
    pub name: String,
    /// Section bytes.
    pub value: Vec<u8>,
}

/// The payload of a seal.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Seal {
    sections: Vec<Section>,
}

impl Seal {
    /// An empty seal.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a section, replacing any with the same name.
    ///
    /// Names are validated because they end up in a file another tool may parse.
    pub fn set(&mut self, name: &str, value: Vec<u8>) -> Result<(), ImageError> {
        if name.is_empty() || name.len() > 64 {
            return Err(ImageError::Manifest(format!(
                "section name {name:?} is empty or too long"
            )));
        }
        if !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(ImageError::Manifest(format!(
                "section name {name:?} has characters outside [A-Za-z0-9-_]"
            )));
        }

        if let Some(existing) = self.sections.iter_mut().find(|s| s.name == name) {
            existing.value = value;
        } else {
            self.sections.push(Section {
                name: name.to_owned(),
                value,
            });
        }

        Ok(())
    }

    /// Add a section whose value is text.
    pub fn set_text(&mut self, name: &str, value: &str) -> Result<(), ImageError> {
        self.set(name, value.as_bytes().to_vec())
    }

    /// Read a section.
    pub fn section(&self, name: &str) -> Option<&[u8]> {
        self.sections
            .iter()
            .find(|section| section.name == name)
            .map(|section| section.value.as_slice())
    }

    /// Read a section as text.
    pub fn text(&self, name: &str) -> Option<String> {
        self.section(name)
            .and_then(|value| String::from_utf8(value.to_vec()).ok())
    }

    /// Every section, in the order they were added.
    pub fn sections(&self) -> &[Section] {
        &self.sections
    }

    /// Serialise the payload.
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&(self.sections.len() as u32).to_le_bytes());

        for section in &self.sections {
            bytes.extend_from_slice(&(section.name.len() as u16).to_le_bytes());
            bytes.extend_from_slice(section.name.as_bytes());
            bytes.extend_from_slice(&(section.value.len() as u64).to_le_bytes());
            bytes.extend_from_slice(&section.value);
        }

        bytes
    }

    /// Parse a payload, bounds-checking every length it declares.
    pub fn decode(bytes: &[u8]) -> Result<Self, ImageError> {
        let mut cursor = 0usize;

        let count = take_u32(bytes, &mut cursor)? as usize;
        // A count larger than the remaining bytes could hold is a lie; refuse before
        // reserving anything for it.
        if count > bytes.len() / 3 {
            return Err(ImageError::Malformed(format!(
                "seal claims {count} sections, which cannot fit in {} bytes",
                bytes.len()
            )));
        }

        let mut seal = Self::new();
        for _ in 0..count {
            let name_len = take_u16(bytes, &mut cursor)? as usize;
            let name_bytes = take(bytes, &mut cursor, name_len)?;
            let name = String::from_utf8(name_bytes.to_vec())
                .map_err(|_| ImageError::Malformed("section name is not UTF-8".to_owned()))?;

            let value_len = take_u64(bytes, &mut cursor)?;
            let value_len = usize::try_from(value_len)
                .map_err(|_| ImageError::Malformed("section length does not fit".to_owned()))?;
            let value = take(bytes, &mut cursor, value_len)?.to_vec();

            seal.set(&name, value)?;
        }

        Ok(seal)
    }
}

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

fn take<'a>(bytes: &'a [u8], cursor: &mut usize, length: usize) -> Result<&'a [u8], ImageError> {
    let end = cursor
        .checked_add(length)
        .ok_or_else(|| ImageError::Malformed("section length overflows".to_owned()))?;

    let slice = bytes
        .get(*cursor..end)
        .ok_or_else(|| ImageError::Malformed("seal ended before its declared length".to_owned()))?;

    *cursor = end;
    Ok(slice)
}

fn take_u16(bytes: &[u8], cursor: &mut usize) -> Result<u16, ImageError> {
    let slice = take(bytes, cursor, 2)?;
    Ok(u16::from_le_bytes([slice[0], slice[1]]))
}

fn take_u32(bytes: &[u8], cursor: &mut usize) -> Result<u32, ImageError> {
    let slice = take(bytes, cursor, 4)?;
    Ok(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

fn take_u64(bytes: &[u8], cursor: &mut usize) -> Result<u64, ImageError> {
    let slice = take(bytes, cursor, 8)?;
    let mut field = [0u8; 8];
    field.copy_from_slice(slice);
    Ok(u64::from_le_bytes(field))
}
