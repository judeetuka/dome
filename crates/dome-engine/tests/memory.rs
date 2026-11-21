use dome_core::MemorySize;
use dome_engine::EngineError;
use dome_engine::memory::GuestMemory;

#[test]
fn allocates_the_requested_size() -> Result<(), Box<dyn std::error::Error>> {
    let memory = GuestMemory::allocate(MemorySize::from_mib(64)?)?;

    assert_eq!(memory.size_bytes(), 64 * 1024 * 1024);
    assert_eq!(memory.size().as_mib(), 64);
    Ok(())
}

#[test]
fn write_then_read_round_trips() -> Result<(), Box<dyn std::error::Error>> {
    let memory = GuestMemory::allocate(MemorySize::from_mib(16)?)?;
    let payload = b"dome writes here";

    memory.write(0x1000, payload)?;
    assert_eq!(memory.read(0x1000, payload.len())?, payload);

    Ok(())
}

#[test]
fn unread_region_reads_as_zero() -> Result<(), Box<dyn std::error::Error>> {
    let memory = GuestMemory::allocate(MemorySize::from_mib(16)?)?;

    assert_eq!(memory.read(0x8000, 8)?, vec![0u8; 8]);
    Ok(())
}

#[test]
fn write_past_the_end_is_refused() -> Result<(), Box<dyn std::error::Error>> {
    let memory = GuestMemory::allocate(MemorySize::from_mib(1)?)?;
    let limit = memory.size_bytes();

    // One byte past the region must fail, not wrap or grow.
    let error = memory.write(limit, b"x").unwrap_err();
    assert!(matches!(error, EngineError::Memory(_)), "got {error:?}");
    Ok(())
}

#[test]
fn read_past_the_end_is_refused() -> Result<(), Box<dyn std::error::Error>> {
    let memory = GuestMemory::allocate(MemorySize::from_mib(1)?)?;

    let error = memory.read(memory.size_bytes() - 4, 8).unwrap_err();
    assert!(matches!(error, EngineError::Memory(_)), "got {error:?}");
    Ok(())
}

#[test]
fn address_overflow_is_a_typed_error() -> Result<(), Box<dyn std::error::Error>> {
    let memory = GuestMemory::allocate(MemorySize::from_mib(1)?)?;

    let error = memory.write(u64::MAX - 2, &[1, 2, 3, 4]).unwrap_err();
    assert!(matches!(error, EngineError::Memory(_)), "got {error:?}");
    Ok(())
}

#[test]
fn host_address_is_stable_and_nonzero() -> Result<(), Box<dyn std::error::Error>> {
    let memory = GuestMemory::allocate(MemorySize::from_mib(8)?)?;

    let first = memory.host_address()?;
    assert_ne!(first, 0);
    assert_eq!(first, memory.host_address()?);
    Ok(())
}
