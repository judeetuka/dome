use std::path::{Path, PathBuf};

use dome_core::{FoundationName, StorageSize};

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum FoundationFormat {
    DomeFoundation,
    EnginePreparedRaw,
    EnginePreparedSparse,
    EnginePreparedVhdx,
}

impl FoundationFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::DomeFoundation => ".foundation.dome",
            Self::EnginePreparedRaw => ".raw.cache",
            Self::EnginePreparedSparse => ".sparse.cache",
            Self::EnginePreparedVhdx => ".vhdx.cache",
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum SealMode {
    Cold,
    SentinelAssisted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundationManifest {
    name: FoundationName,
    format: FoundationFormat,
    virtual_size: StorageSize,
    base_path: PathBuf,
}

impl FoundationManifest {
    pub fn new(
        name: FoundationName,
        format: FoundationFormat,
        virtual_size: StorageSize,
        base_path: impl Into<PathBuf>,
    ) -> Self {
        Self {
            name,
            format,
            virtual_size,
            base_path: base_path.into(),
        }
    }

    pub fn name(&self) -> &FoundationName {
        &self.name
    }

    pub fn format(&self) -> FoundationFormat {
        self.format
    }

    pub fn virtual_size(&self) -> StorageSize {
        self.virtual_size
    }

    pub fn base_path(&self) -> &Path {
        &self.base_path
    }
}
