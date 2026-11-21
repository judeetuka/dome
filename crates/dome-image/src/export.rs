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
use crate::tier::{HostProtection, SecrecyPolicy};
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

/// Section recording the protection tier of the host that produced the package.
pub const SECTION_PROTECTION: &str = "protection";
/// Section marking a package produced knowingly below its desired tier.
pub const SECTION_DEGRADED: &str = "degraded";

/// Layout version of an export payload.
pub const EXPORT_FORMAT: &str = "dome-export/1";

/// The portable name used in place of a host path.
pub const PORTABLE_BASE: &str = "foundation.img";

/// Everything about a package that is not its contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportMeta {
    /// The foundation the chamber was built from.
    pub foundation: String,
    /// Creation time, seconds since the Unix epoch.
    pub created_unix: u64,
    /// The tier of the host producing the package.
    pub host_protection: HostProtection,
    /// The tier of an embedded seal, which the caller must declare because the seal is
    /// encrypted and cannot be read back here.
    pub seal_protection: Option<HostProtection>,
    /// What to do when the host cannot meet the desired tier.
    pub policy: SecrecyPolicy,
}

impl ExportMeta {
    /// Metadata for a host that cannot protect guest memory.
    pub fn new(foundation: impl Into<String>, created_unix: u64) -> Self {
        Self {
            foundation: foundation.into(),
            created_unix,
            host_protection: HostProtection::UNPROTECTED,
            seal_protection: None,
            policy: SecrecyPolicy::AllowDegraded,
        }
    }

    /// Record the tier of the seal being packaged.
    pub fn with_seal_protection(mut self, tier: HostProtection) -> Self {
        self.seal_protection = Some(tier);
        self
    }

    /// Demand confidential guest memory, refusing to package otherwise.
    pub fn requiring_confidentiality(mut self) -> Self {
        self.policy = SecrecyPolicy::RequireConfidential;
        self
    }

    /// Declare this host's tier.
    pub fn on_host(mut self, protection: HostProtection) -> Self {
        self.host_protection = protection;
        self
    }
}

/// An export package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Export {
    bundle: Bundle,
}

impl Export {
    /// Build an export for a host that could not protect guest memory.
    ///
    /// The tier is recorded either way, so a package never leaves the question open.
    pub fn new(
        cell_toml: &str,
        foundation: &str,
        seal: Option<Vec<u8>>,
        logs: &str,
        created_unix: u64,
    ) -> Result<Self, ImageError> {
        Self::with_meta(
            cell_toml,
            seal,
            logs,
            ExportMeta::new(foundation, created_unix),
        )
    }

    /// Build an export, enforcing the protection policy.
    ///
    /// Refuses in two cases: a host that cannot meet the policy, and an embedded seal that
    /// claims more protection than the host producing the package can offer. A package must
    /// not understate what it carries, which is the whole point of recording the tier.
    pub fn with_meta(
        cell_toml: &str,
        seal: Option<Vec<u8>>,
        logs: &str,
        meta: ExportMeta,
    ) -> Result<Self, ImageError> {
        let protection = meta.host_protection;
        let policy = meta.policy;

        // Scanned before assembly: a package that leaks a home directory is worse than one
        // that refuses to build.
        reject_host_paths(SECTION_CELL, cell_toml)?;
        reject_host_paths(SECTION_LOGS, logs)?;

        if !policy.permits(protection) {
            return Err(ImageError::Manifest(format!(
                "this host is {} but the export policy requires confidential guest memory",
                protection.to_record()
            )));
        }

        // A package must not claim less protection than the seal it carries, or it would
        // launder a confidential chamber into a plain one. The seal is encrypted, so its tier
        // cannot be read back here without the key: the caller declares it, and the caller is
        // the one that just produced it.
        match (&seal, meta.seal_protection) {
            (Some(_), None) => {
                return Err(ImageError::Manifest(
                    "a seal is being packaged without declaring its protection tier".to_owned(),
                ));
            }
            (Some(_), Some(seal_tier)) => {
                if seal_tier.is_confidential() && !protection.is_confidential() {
                    return Err(ImageError::Manifest(format!(
                        "the packaged seal claims {} protection but this host is {}",
                        seal_tier.to_record(),
                        protection.to_record()
                    )));
                }
            }
            (None, _) => {}
        }

        let mut bundle = Bundle::new();
        bundle.set_text(SECTION_FORMAT, EXPORT_FORMAT)?;
        bundle.set_text(SECTION_PROTECTION, &protection.to_record())?;
        if matches!(policy, SecrecyPolicy::AllowDegraded) && !protection.is_confidential() {
            bundle.set_text(SECTION_DEGRADED, "true")?;
        }
        bundle.set_text(SECTION_CELL, cell_toml)?;
        bundle.set_text(SECTION_FOUNDATION, &meta.foundation)?;
        bundle.set_text(SECTION_CREATED, &meta.created_unix.to_string())?;
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

    /// The protection tier the package records.
    pub fn protection(&self) -> Result<HostProtection, ImageError> {
        let value = self
            .bundle
            .text(SECTION_PROTECTION)
            .ok_or_else(|| ImageError::Manifest("export records no protection tier".to_owned()))?;

        HostProtection::parse_record(&value)
            .ok_or_else(|| ImageError::Manifest(format!("unknown protection tier {value:?}")))
    }

    /// Whether the package was produced knowingly below its desired tier.
    pub fn is_degraded(&self) -> bool {
        self.bundle.text(SECTION_DEGRADED).as_deref() == Some("true")
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
