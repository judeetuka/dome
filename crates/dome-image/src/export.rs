//! Export packages: a chamber as one file that can move to another host.
//!
//! An export carries the cell blueprint, the foundation it was built from, its seal and its
//! logs. Everything sits inside one signed, encrypted container, so the package is
//! self-describing and travels without side files.
//!
//! **Nothing host-specific may travel with it.** The container manifest's own base path is
//! replaced with a portable name, and the blueprint and logs are scanned for absolute paths,
//! because an export that leaks `/home/<user>/...` onto another machine is both a privacy
//! problem and a broken package.

use crate::FoundationManifest;
use crate::bundle::Bundle;
use crate::image_error::ImageError;
use crate::reader::{ContainerVerifier, ReadContainer, read_container_with};
use crate::writer::write_container_suite2;

/// Marks the package as an export and names its layout version.
pub const SECTION_FORMAT: &str = "format";
/// The cell blueprint, as TOML.
pub const SECTION_CELL: &str = "cell";
/// The foundation name the chamber was built from.
pub const SECTION_FOUNDATION: &str = "foundation";
/// The chamber's seal, as a complete `.seal.dome` container.
pub const SECTION_SEAL: &str = "seal";
/// The console log.
pub const SECTION_LOGS: &str = "logs";
/// Creation time, in seconds since the Unix epoch.
pub const SECTION_CREATED: &str = "created-unix";

/// Layout version of an export payload.
pub const EXPORT_FORMAT: &str = "dome-export/1";

/// The portable name used in place of a host path.
pub const PORTABLE_BASE: &str = "foundation.img";

/// An export package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Export {
    bundle: Bundle,
}

impl Export {
    /// Build an export, refusing anything that would leak a host path.
    pub fn new(
        cell_toml: &str,
        foundation: &str,
        seal: Option<Vec<u8>>,
        logs: &str,
        created_unix: u64,
    ) -> Result<Self, ImageError> {
        // Scanned before assembly: a package that leaks a home directory is worse than one
        // that refuses to build.
        reject_host_paths(SECTION_CELL, cell_toml)?;
        reject_host_paths(SECTION_LOGS, logs)?;

        let mut bundle = Bundle::new();
        bundle.set_text(SECTION_FORMAT, EXPORT_FORMAT)?;
        bundle.set_text(SECTION_CELL, cell_toml)?;
        bundle.set_text(SECTION_FOUNDATION, foundation)?;
        bundle.set_text(SECTION_CREATED, &created_unix.to_string())?;
        bundle.set_text(SECTION_LOGS, logs)?;

        if let Some(seal) = seal {
            bundle.set(SECTION_SEAL, seal)?;
        }

        Ok(Self { bundle })
    }

    /// The cell blueprint.
    pub fn cell(&self) -> Option<String> {
        self.bundle.text(SECTION_CELL)
    }

    /// The foundation name.
    pub fn foundation(&self) -> Option<String> {
        self.bundle.text(SECTION_FOUNDATION)
    }

    /// The console log.
    pub fn logs(&self) -> Option<String> {
        self.bundle.text(SECTION_LOGS)
    }

    /// The embedded seal, as a complete container.
    pub fn seal(&self) -> Option<&[u8]> {
        self.bundle.section(SECTION_SEAL)
    }

    /// The layout version recorded in the package.
    pub fn format(&self) -> Option<String> {
        self.bundle.text(SECTION_FORMAT)
    }

    /// The payload, for tests and diagnostics.
    pub fn bundle(&self) -> &Bundle {
        &self.bundle
    }

    /// Parse a payload, refusing an unknown layout version.
    pub fn decode(bytes: &[u8]) -> Result<Self, ImageError> {
        let bundle = Bundle::decode(bytes)?;

        match bundle.text(SECTION_FORMAT).as_deref() {
            Some(EXPORT_FORMAT) => Ok(Self { bundle }),
            Some(other) => Err(ImageError::Manifest(format!(
                "export format {other:?} is not understood by this build"
            ))),
            None => Err(ImageError::Manifest(
                "payload is not an export: no format marker".to_owned(),
            )),
        }
    }
}

/// Write an export as a suite-2 container.
///
/// The foundation manifest is rewritten with a portable base path, so the host path of the
/// local foundation does not travel with the package.
pub fn write_export(
    export: &Export,
    manifest: &FoundationManifest,
    file_key: &[u8; 32],
    ed25519: &dome_crypto::Ed25519KeyPair,
    mldsa: &dome_crypto::pq::PqdsaKeyPair,
) -> Result<Vec<u8>, ImageError> {
    let portable = FoundationManifest::new(
        manifest.name().clone(),
        manifest.format(),
        manifest.virtual_size(),
        PORTABLE_BASE,
    );

    write_container_suite2(&portable, &export.bundle.encode(), file_key, ed25519, mldsa)
}

/// Read an export, verifying and decrypting it first.
pub fn read_export(
    bytes: &[u8],
    file_key: &[u8; 32],
    verifier: ContainerVerifier<'_>,
) -> Result<(Export, ReadContainer), ImageError> {
    let container = read_container_with(bytes, file_key, verifier)?;
    let export = Export::decode(&container.payload)?;
    Ok((export, container))
}

/// Refuse a section whose text contains an absolute host path.
fn reject_host_paths(section: &str, text: &str) -> Result<(), ImageError> {
    if let Some(found) = find_host_path(text) {
        return Err(ImageError::Manifest(format!(
            "section {section:?} contains the host path {found:?}; an export must not carry one"
        )));
    }

    Ok(())
}

/// Find the first absolute path that looks specific to a host.
///
/// Deliberately narrow: a container image legitimately mentions paths like `/bin/sh` or
/// `/etc/hosts`, so only locations that identify *this* machine are refused.
pub fn find_host_path(text: &str) -> Option<String> {
    const HOST_PREFIXES: [&str; 7] = [
        "/home/",
        "/root/",
        "/Users/",
        "/var/lib/dome/",
        "/run/user/",
        "/tmp/",
        "/mnt/",
    ];

    for prefix in HOST_PREFIXES {
        if let Some(index) = text.find(prefix) {
            let tail = &text[index..];
            let end = tail
                .find(['\n', '\r', '\t', ' ', '"', '\'', ')'])
                .unwrap_or(tail.len());
            return Some(tail[..end].to_owned());
        }
    }

    None
}
