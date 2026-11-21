//! A payload of named sections, used by both seals and exports.
//!
//! Sections rather than a struct, for two reasons: a field can be added later without
//! changing what an old artifact means, and a reader that does not recognise a section can
//! still open the rest.

use crate::image_error::ImageError;

/// A named piece of a bundle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    /// Section name.
    pub name: String,
    /// Section bytes.
    pub value: Vec<u8>,
}

/// An ordered set of named sections.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Bundle {
    sections: Vec<Section>,
}

impl Bundle {
    /// An empty bundle.
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

        if !name.chars().all(|character| {
            character.is_ascii_alphanumeric() || character == '-' || character == '_'
        }) {
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
                "bundle claims {count} sections, which cannot fit in {} bytes",
                bytes.len()
            )));
        }

        let mut bundle = Self::new();
        for _ in 0..count {
            let name_len = take_u16(bytes, &mut cursor)? as usize;
            let name = String::from_utf8(take(bytes, &mut cursor, name_len)?.to_vec())
                .map_err(|_| ImageError::Malformed("section name is not UTF-8".to_owned()))?;

            let value_len = usize::try_from(take_u64(bytes, &mut cursor)?)
                .map_err(|_| ImageError::Malformed("section length does not fit".to_owned()))?;
            let value = take(bytes, &mut cursor, value_len)?.to_vec();

            bundle.set(&name, value)?;
        }

        Ok(bundle)
    }
}

fn take<'a>(bytes: &'a [u8], cursor: &mut usize, length: usize) -> Result<&'a [u8], ImageError> {
    let end = cursor
        .checked_add(length)
        .ok_or_else(|| ImageError::Malformed("section length overflows".to_owned()))?;

    let slice = bytes.get(*cursor..end).ok_or_else(|| {
        ImageError::Malformed("bundle ended before its declared length".to_owned())
    })?;

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
