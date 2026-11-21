use dome_core::MemorySize;
use dome_engine::EngineError;
use dome_engine::loader::{MIN_KERNEL_ADDRESS, load_bzimage};
use dome_engine::memory::GuestMemory;
use dome_test_utils::test_fixture_path;

fn kernel_fixture() -> std::path::PathBuf {
    test_fixture_path("kernel/vmlinuz-virt")
}

#[test]
fn loads_the_real_kernel_fixture() -> Result<(), Box<dyn std::error::Error>> {
    let path = kernel_fixture();
    if !path.is_file() {
        eprintln!("skipping: run scripts/download-test-kernel.sh to fetch the fixture");
        return Ok(());
    }

    let memory = GuestMemory::allocate(MemorySize::from_mib(512)?)?;
    let loaded = load_bzimage(&memory, &path, MIN_KERNEL_ADDRESS)?;

    assert_eq!(loaded.entry.0, MIN_KERNEL_ADDRESS);

    let header = loaded
        .setup_header
        .expect("bzImage must expose a setup header");
    // setup_header is packed: copy the field into a local instead of taking a reference.
    let protocol_version = header.version;
    assert!(
        protocol_version >= 0x0200,
        "boot protocol too old: {protocol_version:#x}"
    );
    assert!(loaded.kernel_end > MIN_KERNEL_ADDRESS);
    Ok(())
}

#[test]
fn refuses_a_kernel_below_real_mode() -> Result<(), Box<dyn std::error::Error>> {
    let memory = GuestMemory::allocate(MemorySize::from_mib(64)?)?;

    let error = load_bzimage(&memory, &kernel_fixture(), 0x7000).unwrap_err();
    assert!(matches!(error, EngineError::Setup(_)), "got {error:?}");
    Ok(())
}

#[test]
fn refuses_an_unaligned_kernel_start() -> Result<(), Box<dyn std::error::Error>> {
    let memory = GuestMemory::allocate(MemorySize::from_mib(64)?)?;

    let error = load_bzimage(&memory, &kernel_fixture(), MIN_KERNEL_ADDRESS + 1).unwrap_err();
    assert!(matches!(error, EngineError::Setup(_)), "got {error:?}");
    Ok(())
}

#[test]
fn missing_kernel_file_is_a_typed_error() -> Result<(), Box<dyn std::error::Error>> {
    let memory = GuestMemory::allocate(MemorySize::from_mib(64)?)?;

    let error = load_bzimage(
        &memory,
        std::path::Path::new("/nonexistent/vmlinuz"),
        MIN_KERNEL_ADDRESS,
    )
    .unwrap_err();
    assert!(matches!(error, EngineError::Setup(_)), "got {error:?}");
    Ok(())
}

#[test]
fn refuses_a_kernel_that_cannot_fit_with_its_workspace() -> Result<(), Box<dyn std::error::Error>> {
    // 16 MiB is smaller than this kernel's declared init_size, so the decompressor would
    // have nowhere to expand. The loader must say so instead of letting the guest die
    // silently with no console output.
    let memory = GuestMemory::allocate(MemorySize::from_mib(16)?)?;

    let result = load_bzimage(&memory, &kernel_fixture(), MIN_KERNEL_ADDRESS);
    match result {
        Err(EngineError::Memory(_)) => Ok(()),
        Err(other) => panic!("expected a Memory error, got {other:?}"),
        Ok(_) => panic!("loading a kernel into 16 MiB should have been refused"),
    }
}
