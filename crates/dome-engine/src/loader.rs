//! Kernel loading.
//!
//! Only bzImage for now. The surface is deliberately small: a loader parses untrusted file
//! data, so the less it understands the better (ADR 0002).

use std::io::Cursor;
use std::path::Path;

use linux_loader::loader::KernelLoader;
use linux_loader::loader::bzimage;
use vm_memory::GuestAddress;

use crate::EngineError;
use crate::memory::GuestMemory;

/// Lowest address a kernel may be loaded at. Below 1 MiB is real-mode space.
pub const MIN_KERNEL_ADDRESS: u64 = 0x0010_0000;

/// A kernel image placed in guest memory.
pub struct LoadedKernel {
    /// Where the kernel was loaded, and where execution starts.
    pub entry: GuestAddress,
    /// Parsed setup header, needed to build the zero page.
    pub setup_header: Option<linux_loader::loader::bootparam::setup_header>,
    /// First address past the loaded image.
    pub kernel_end: u64,
}

impl std::fmt::Debug for LoadedKernel {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // setup_header is #[repr(packed)]: copy fields to locals before formatting them,
        // because forming a reference to a packed field is undefined behaviour.
        let protocol = self.setup_header.as_ref().map(|header| {
            let version = header.version;
            let init_size = header.init_size;
            (version, init_size)
        });

        formatter
            .debug_struct("LoadedKernel")
            .field("entry", &self.entry)
            .field("kernel_end", &self.kernel_end)
            .field("boot_protocol", &protocol.map(|(version, _)| version))
            .field("init_size", &protocol.map(|(_, init_size)| init_size))
            .finish()
    }
}

/// Load a bzImage into guest memory at the given start address.
///
/// The start address must be page aligned and at or above MIN_KERNEL_ADDRESS. The image and
/// the decompression workspace it declares (init_size) must both fit in guest RAM; a
/// workspace that does not fit makes the guest die silently inside the decompressor, which
/// is exactly the failure this check converts into a clear error.
pub fn load_bzimage(
    memory: &GuestMemory,
    path: &Path,
    kernel_start: u64,
) -> Result<LoadedKernel, EngineError> {
    if kernel_start < MIN_KERNEL_ADDRESS {
        return Err(EngineError::Setup(format!(
            "kernel start {kernel_start:#x} is below the {MIN_KERNEL_ADDRESS:#x} minimum"
        )));
    }

    if !kernel_start.is_multiple_of(0x1000) {
        return Err(EngineError::Setup(format!(
            "kernel start {kernel_start:#x} is not page aligned"
        )));
    }

    let image = std::fs::read(path)
        .map_err(|error| EngineError::Setup(format!("cannot read {}: {error}", path.display())))?;

    if image.is_empty() {
        return Err(EngineError::Setup(format!(
            "kernel image {} is empty",
            path.display()
        )));
    }

    let mut cursor = Cursor::new(&image);

    // None for highmem_start_address: let the image's own code32_start decide, and skip the
    // loader check that rejects a kernel starting below high memory.
    let loaded = bzimage::BzImage::load(
        memory.map(),
        Some(GuestAddress(kernel_start)),
        &mut cursor,
        None,
    )
    .map_err(|error| EngineError::Setup(format!("bzImage load failed: {error}")))?;

    let kernel_end = loaded.kernel_end;
    let limit = memory.size_bytes();

    if kernel_end > limit {
        return Err(EngineError::Memory(format!(
            "kernel ends at {kernel_end:#x}, past {limit:#x} of guest memory"
        )));
    }

    if let Some(header) = loaded.setup_header {
        let init_size = header.init_size as u64;
        let workspace_end = kernel_start.saturating_add(init_size);

        if init_size > 0 && workspace_end > limit {
            return Err(EngineError::Memory(format!(
                "kernel needs {init_size:#x} bytes of decompression workspace from {kernel_start:#x}, past {limit:#x} of guest memory"
            )));
        }
    }

    Ok(LoadedKernel {
        entry: loaded.kernel_load,
        setup_header: loaded.setup_header,
        kernel_end,
    })
}
