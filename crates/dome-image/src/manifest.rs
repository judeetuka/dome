//! Serialising foundation metadata for storage inside the encrypted manifest.
//!
//! This format lives entirely inside authenticated ciphertext, so it is not parsed by anything
//! that has not already proved it is trusted. It is still bounds-checked on read: a decrypted
//! manifest that lies about its own lengths should be a typed error, not a panic.

use dome_core::{FoundationName, StorageSize};

use crate::FoundationFormat;
use crate::image_error::ImageError;

/// Encode a foundation manifest.
pub fn encode_manifest(
    name: &FoundationName,
    format: FoundationFormat,
    virtual_size: StorageSize,
    base_path: &str,
) -> Result<Vec<u8>, ImageError> {
    let name_bytes = name.as_str().as_bytes();
    let base_bytes = base_path.as_bytes();

    if name_bytes.len() > u16::MAX as usize {
        return Err(ImageError::Manifest(
            "foundation name is too long".to_owned(),
        ));
    }
    if base_bytes.len() > u16::MAX as usize {
        return Err(ImageError::Manifest("base path is too long".to_owned()));
    }

    let mut bytes = Vec::with_capacity(2 + name_bytes.len() + 1 + 8 + 2 + base_bytes.len());
    bytes.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
    bytes.extend_from_slice(name_bytes);
    bytes.push(format.to_wire());
    bytes.extend_from_slice(&virtual_size.as_mib().to_le_bytes());
    bytes.extend_from_slice(&(base_bytes.len() as u16).to_le_bytes());
    bytes.extend_from_slice(base_bytes);

    Ok(bytes)
}

/// A decoded manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedManifest {
    /// Foundation name.
    pub name: FoundationName,
    /// Storage format.
    pub format: FoundationFormat,
    /// Virtual size.
    pub virtual_size: StorageSize,
    /// Path to the base artifact.
    pub base_path: String,
}

/// Decode a manifest written by [encode_manifest].
pub fn decode_manifest(bytes: &[u8]) -> Result<DecodedManifest, ImageError> {
    let mut cursor = 0usize;

    let name_len = take_u16(bytes, &mut cursor)? as usize;
    let name = take_str(bytes, &mut cursor, name_len)?;
    let format = FoundationFormat::from_wire(take_u8(bytes, &mut cursor)?)
        .map_err(|error| ImageError::Manifest(format!("bad format byte: {error}")))?;
    let virtual_mib = take_u64(bytes, &mut cursor)?;
    let base_len = take_u16(bytes, &mut cursor)? as usize;
    let base_path = take_str(bytes, &mut cursor, base_len)?;

    Ok(DecodedManifest {
        name: FoundationName::new(&name)
            .map_err(|error| ImageError::Manifest(format!("bad foundation name: {error}")))?,
        format,
        virtual_size: StorageSize::from_mib(virtual_mib)
            .map_err(|error| ImageError::Manifest(format!("bad virtual size: {error}")))?,
        base_path,
    })
}

fn take_u8(bytes: &[u8], cursor: &mut usize) -> Result<u8, ImageError> {
    let value = *bytes
        .get(*cursor)
        .ok_or_else(|| ImageError::Manifest("manifest ended early".to_owned()))?;
    *cursor += 1;
    Ok(value)
}

fn take_u16(bytes: &[u8], cursor: &mut usize) -> Result<u16, ImageError> {
    let slice = bytes
        .get(*cursor..*cursor + 2)
        .ok_or_else(|| ImageError::Manifest("manifest ended early".to_owned()))?;
    *cursor += 2;
    Ok(u16::from_le_bytes([slice[0], slice[1]]))
}

fn take_u64(bytes: &[u8], cursor: &mut usize) -> Result<u64, ImageError> {
    let slice = bytes
        .get(*cursor..*cursor + 8)
        .ok_or_else(|| ImageError::Manifest("manifest ended early".to_owned()))?;
    *cursor += 8;
    let mut field = [0u8; 8];
    field.copy_from_slice(slice);
    Ok(u64::from_le_bytes(field))
}

fn take_str(bytes: &[u8], cursor: &mut usize, length: usize) -> Result<String, ImageError> {
    let slice = bytes
        .get(*cursor..*cursor + length)
        .ok_or_else(|| ImageError::Manifest("manifest ended early".to_owned()))?;
    *cursor += length;

    String::from_utf8(slice.to_vec())
        .map_err(|error| ImageError::Manifest(format!("manifest text is not UTF-8: {error}")))
}
