//! Which protection level a chamber ran under, recorded so it cannot be claimed later.
//!
//! Researched rather than assumed:
//!
//! | Platform | Confidential guest memory | Requires |
//! | --- | --- | --- |
//! | Linux | Intel TDX, AMD SEV-SNP | CPU support and a host kernel that enables it |
//! | Windows | Shielded VM (vTPM, BitLocker-encrypted, HGS attestation) | a guarded fabric with a Host Guardian Service |
//! | macOS | **none exposed** | Virtualization.framework offers no guest memory confidentiality |
//!
//! **What this is not.** A seal is encrypted at rest on every tier; the file encryption is
//! what protects it, and TDX adds nothing to a file sitting on a disk. What the tier records
//! is whether the guest's memory was hardware-protected **while it was running**, which is
//! exactly the window in which a host root could have copied it before it was sealed.

use std::fmt;

/// How well the host that produced an artifact could keep a guest's memory private.
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum ProtectionTier {
    /// Nothing isolated guest memory from the host: a host root could read it at any time.
    Unprotected,
    /// The device protects guest memory against the host while it runs.
    Confidential,
}

impl ProtectionTier {
    /// Whether a seal made here may be described as confidential.
    pub fn is_confidential(self) -> bool {
        self == Self::Confidential
    }

    /// Stable name for the on-disk record.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unprotected => "unprotected",
            Self::Confidential => "confidential",
        }
    }

    /// Parse the on-disk form.
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "unprotected" => Self::Unprotected,
            "confidential" => Self::Confidential,
            _ => return None,
        })
    }
}

impl fmt::Display for ProtectionTier {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// The mechanism behind a confidential tier, recorded so a reader knows what was claimed.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum ConfidentialMechanism {
    /// Intel Trust Domain Extensions.
    Tdx,
    /// AMD Secure Nested Paging.
    SevSnp,
    /// Windows Shielded VM on a guarded fabric.
    ShieldedVm,
}

impl ConfidentialMechanism {
    /// Stable name for the on-disk record.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tdx => "tdx",
            Self::SevSnp => "sev-snp",
            Self::ShieldedVm => "shielded-vm",
        }
    }

    /// Parse the on-disk form.
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "tdx" => Self::Tdx,
            "sev-snp" => Self::SevSnp,
            "shielded-vm" => Self::ShieldedVm,
            _ => return None,
        })
    }
}

/// A complete statement about where an artifact was produced.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct HostProtection {
    /// The tier.
    pub tier: ProtectionTier,
    /// The mechanism, present exactly when the tier is confidential.
    pub mechanism: Option<ConfidentialMechanism>,
}

impl HostProtection {
    /// A host that could not isolate guest memory.
    pub const UNPROTECTED: Self = Self {
        tier: ProtectionTier::Unprotected,
        mechanism: None,
    };

    /// A host that could.
    pub fn confidential(mechanism: ConfidentialMechanism) -> Self {
        Self {
            tier: ProtectionTier::Confidential,
            mechanism: Some(mechanism),
        }
    }

    /// Whether this lands in the confidential tier.
    pub fn is_confidential(self) -> bool {
        self.tier.is_confidential() && self.mechanism.is_some()
    }

    /// Encode for a record field.
    ///
    /// The unprotected case reads as a word rather than as an absence, because this string ends
    /// up inside error messages an operator has to understand.
    pub fn to_record(self) -> String {
        match self.mechanism {
            Some(mechanism) => mechanism.as_str().to_owned(),
            None => "unprotected".to_owned(),
        }
    }

    /// Parse a record field.
    pub fn parse_record(value: &str) -> Option<Self> {
        // "none" is accepted because records written by an earlier build used it.
        if value == "unprotected" || value == "none" {
            return Some(Self::UNPROTECTED);
        }

        let mechanism = ConfidentialMechanism::parse(value)?;
        Some(Self::confidential(mechanism))
    }
}

/// What to do when a host cannot keep the promise an artifact wants.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum SecrecyPolicy {
    /// Refuse to produce or resume a confidential artifact on an unprotected host.
    RequireConfidential,
    /// Accept an unprotected host, but mark the artifact as degraded so nobody mistakes it.
    AllowDegraded,
}

impl SecrecyPolicy {
    /// Whether a host at this tier may produce or resume the artifact.
    pub fn permits(self, protection: HostProtection) -> bool {
        match self {
            Self::RequireConfidential => protection.is_confidential(),
            Self::AllowDegraded => true,
        }
    }
}
