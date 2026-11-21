//! Guest memory allocation for a chamber.
//!
//! Wraps vm-memory so the rest of the engine never handles raw mappings. Every access is
//! bounds-checked against the region we own, because the alternative is the kind of bug
//! that turns a malformed guest descriptor into host memory corruption.

use dome_core::MemorySize;
use vm_memory::{Bytes, GuestAddress, GuestMemoryBackend, GuestMemoryMmap};

use crate::EngineError;

/// Upper bound on a single chamber's RAM.
///
/// Not a hardware limit: a sanity ceiling that turns an absurd blueprint into a typed
/// error instead of an allocation failure part-way through boot.
pub const MAX_MEMORY_BYTES: u64 = 1024 * 1024 * 1024 * 1024;

/// The guest's RAM.
#[derive(Debug)]
pub struct GuestMemory {
    inner: GuestMemoryMmap,
    size: MemorySize,
}

impl GuestMemory {
    /// Allocate a contiguous region backing the whole physical address space.
    pub fn allocate(size: MemorySize) -> Result<Self, EngineError> {
        let bytes = size
            .as_mib()
            .checked_mul(1024 * 1024)
            .ok_or_else(|| EngineError::Memory("memory size overflows".to_owned()))?;

        if bytes > MAX_MEMORY_BYTES {
            return Err(EngineError::Memory(format!(
                "{bytes} bytes exceeds the {MAX_MEMORY_BYTES} byte ceiling"
            )));
        }

        let inner = GuestMemoryMmap::from_ranges(&[(GuestAddress(0), bytes as usize)])
            .map_err(|error| EngineError::Memory(format!("allocation failed: {error}")))?;

        Ok(Self { inner, size })
    }

    /// Configured size of the region.
    pub fn size(&self) -> MemorySize {
        self.size
    }

    /// Size in bytes.
    pub fn size_bytes(&self) -> u64 {
        self.size.as_mib() * 1024 * 1024
    }

    /// The underlying map, for kernel loading and KVM setup.
    pub fn map(&self) -> &GuestMemoryMmap {
        &self.inner
    }

    /// Host virtual address of guest offset 0, which is what KVM maps into the VM.
    pub fn host_address(&self) -> Result<u64, EngineError> {
        self.inner
            .get_host_address(GuestAddress(0))
            .map(|pointer| pointer.cast::<u8>() as u64)
            .map_err(|error| EngineError::Memory(format!("no host address: {error}")))
    }

    /// Bounds-check a write against the region we own.
    pub fn write(&self, address: u64, data: &[u8]) -> Result<(), EngineError> {
        self.check_range(address, data.len() as u64, "write")?;

        self.inner
            .write_slice(data, GuestAddress(address))
            .map_err(|error| EngineError::Memory(format!("write failed: {error}")))
    }

    /// Bounds-check a read against the region we own.
    pub fn read(&self, address: u64, length: usize) -> Result<Vec<u8>, EngineError> {
        self.check_range(address, length as u64, "read")?;

        let mut buffer = vec![0u8; length];
        self.inner
            .read_slice(&mut buffer, GuestAddress(address))
            .map_err(|error| EngineError::Memory(format!("read failed: {error}")))?;

        Ok(buffer)
    }

    /// Copy the whole region out, for a seal.
    ///
    /// This allocates the full guest size; a seal of a 512 MiB chamber costs 512 MiB of host
    /// memory for the duration. Streaming to a file would avoid that and is the obvious next
    /// step when seals grow, which is why this is a separate function rather than something
    /// `launch` does.
    pub fn capture(&self) -> Result<Vec<u8>, EngineError> {
        self.read(0, self.size_bytes() as usize)
    }

    fn check_range(&self, address: u64, length: u64, operation: &str) -> Result<(), EngineError> {
        let end = address
            .checked_add(length)
            .ok_or_else(|| EngineError::Memory(format!("{operation} address overflows")))?;

        let limit = self.size_bytes();

        if end > limit {
            return Err(EngineError::Memory(format!(
                "{operation} of {length} bytes at {address:#x} ends at {end:#x}, past {limit:#x}"
            )));
        }

        Ok(())
    }
}
