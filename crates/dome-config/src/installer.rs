//! Where a chamber's packages come from: the chamber itself.
//!
//! A chamber must not depend on the host it happens to run on. Copying a toolchain out of the
//! host looks convenient and is wrong in three ways: the host may be macOS or Windows and have no
//! Linux toolchains at all, whatever it does have is linked against its own C library, and the
//! resulting image would only ever work on the machine that built it.
//!
//! So an image that needs packages carries a **package manager** and installs into itself, over
//! the network, from upstream. The host's only contribution is that one binary, and it has to be
//! statically linked so it cannot drag the host's libraries into the chamber.

use crate::ConfigError;

/// How a chamber installs its own packages.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Installer {
    /// No installer: the image contains exactly what it was built with.
    None,
    /// mise, fetching toolchains from upstream.
    Mise,
    /// apk, fetching Alpine packages.
    Apk,
    /// pacman, fetching Arch packages.
    Pacman,
}

impl Installer {
    /// Parse the field.
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value.trim() {
            "" | "none" => Self::None,
            "mise" => Self::Mise,
            "apk" => Self::Apk,
            "pacman" => Self::Pacman,
            _ => return None,
        })
    }

    /// The name it is written as.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Mise => "mise",
            Self::Apk => "apk",
            Self::Pacman => "pacman",
        }
    }

    /// Whether an image with this installer needs one shipped inside it.
    pub fn needs_binary(self) -> bool {
        self != Self::None
    }
}

/// Whether an ELF image is statically linked.
///
/// A static binary has no `PT_INTERP` program header, because there is no dynamic loader to name.
/// That is exactly the property an embedded installer must have: a dynamically linked one would
/// require the host's C library inside the chamber, which is the dependency being avoided.
pub fn is_static_elf(bytes: &[u8]) -> Result<bool, ConfigError> {
    if bytes.len() < 64 || &bytes[0..4] != b"\x7fELF" {
        return Err(ConfigError::InvalidValue {
            field: "installer",
            reason: "not an ELF binary, so it cannot be executed in a chamber".to_owned(),
        });
    }

    // 64-bit little-endian is the only case that matters for the engines we ship.
    if bytes[4] != 2 || bytes[5] != 1 {
        return Err(ConfigError::InvalidValue {
            field: "installer",
            reason: "only 64-bit little-endian ELF binaries are supported".to_owned(),
        });
    }

    let program_offset =
        u64::from_le_bytes(
            bytes[32..40]
                .try_into()
                .map_err(|_| ConfigError::InvalidValue {
                    field: "installer",
                    reason: "truncated ELF header".to_owned(),
                })?,
        ) as usize;
    let entry_size =
        u16::from_le_bytes(
            bytes[54..56]
                .try_into()
                .map_err(|_| ConfigError::InvalidValue {
                    field: "installer",
                    reason: "truncated ELF header".to_owned(),
                })?,
        ) as usize;
    let entry_count =
        u16::from_le_bytes(
            bytes[56..58]
                .try_into()
                .map_err(|_| ConfigError::InvalidValue {
                    field: "installer",
                    reason: "truncated ELF header".to_owned(),
                })?,
        ) as usize;

    // PT_INTERP is program header type 3.
    const PT_INTERP: u32 = 3;

    for index in 0..entry_count {
        let start = program_offset + index * entry_size;
        let Some(header) = bytes.get(start..start + entry_size) else {
            return Err(ConfigError::InvalidValue {
                field: "installer",
                reason: "truncated program header".to_owned(),
            });
        };

        let kind = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
        if kind == PT_INTERP {
            return Ok(false);
        }
    }

    Ok(true)
}
