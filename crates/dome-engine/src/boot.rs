//! Building the zero page the kernel expects.
//!
//! Everything here was learned the hard way during the T59 spike and is now engine code.
//! Three of these fields are load-bearing in ways nobody guesses: type_of_loader gates
//! whether the kernel reads the initrd at all, the e820 map is what lets the decompressor
//! plan its workspace, and loadflags decides which boot protocol the kernel honours.

use dome_core::MemorySize;
use linux_loader::loader::bootparam::setup_header;
use vm_memory::GuestAddress;

use crate::EngineError;
use crate::memory::GuestMemory;

/// Traditional real-mode zero page used by the 32-bit boot protocol.
pub const ZERO_PAGE_ADDRESS: u64 = 0x0000_7000;
/// Offset of the setup header inside the zero page.
const SETUP_HEADER_OFFSET: u64 = 0x01F1;
/// Offset of the e820 entry count inside the zero page.
const E820_ENTRIES_OFFSET: u64 = 0x01E8;
/// Offset of the e820 table inside the zero page.
const E820_TABLE_OFFSET: u64 = 0x02D0;
/// One e820 record: 8-byte base, 8-byte length, 4-byte type.
const E820_RECORD_SIZE: usize = 20;

/// Where the command line is placed.
pub const CMDLINE_ADDRESS: u64 = 0x0002_0000;
/// Maximum command line length the kernel accepts.
pub const CMDLINE_CAPACITY: usize = 2048;

/// The preferred initramfs address, used when the guest has room for it.
pub const INITRD_ADDRESS: u64 = 0x1000_0000;

/// Keep this much room between the initramfs and the top of RAM.
const INITRD_MARGIN: u64 = 0x10_0000;

/// Where to put an initramfs in this chamber.
///
/// The preferred address is used when it fits, and otherwise the archive goes as high as it can.
/// A fixed address meant any chamber smaller than 256 MiB could not carry an initramfs at all,
/// which is exactly backwards: the leanest chambers are the ones most likely to want one.
pub fn initrd_address(memory_size: u64, initrd_size: usize) -> Result<u64, crate::EngineError> {
    let size = initrd_size as u64;
    if size == 0 {
        return Err(crate::EngineError::Setup(
            "an empty initramfs cannot be placed".to_owned(),
        ));
    }

    let ceiling = memory_size
        .saturating_sub(size)
        .saturating_sub(INITRD_MARGIN);
    let address = INITRD_ADDRESS.min(ceiling) & !0xFFF;

    // The kernel is loaded at MIN_KERNEL_ADDRESS and decompresses upward from there.
    let floor = crate::loader::MIN_KERNEL_ADDRESS + size;
    if address <= floor {
        return Err(crate::EngineError::Setup(format!(
            "a {size}-byte initramfs does not fit in {memory_size} bytes of guest memory"
        )));
    }

    Ok(address)
}

/// A placed initramfs: address and length.
#[derive(Debug, Clone, Copy)]
pub struct PlacedInitrd {
    pub address: u64,
    pub size: usize,
}

/// Write the zero page and return its address, which is what ESI must point at.
///
/// Deliberately takes the header by value: `setup_header` is `#[repr(packed)]`, so copying
/// it locally avoids ever forming a misaligned reference to a field.
pub fn write_zero_page(
    memory: &GuestMemory,
    mut header: setup_header,
    cmdline: &str,
    initrd: Option<PlacedInitrd>,
    kernel_start: u64,
) -> Result<GuestAddress, EngineError> {
    if cmdline.len() + 1 > CMDLINE_CAPACITY {
        return Err(EngineError::Setup(format!(
            "command line is {} bytes, limit is {CMDLINE_CAPACITY}",
            cmdline.len()
        )));
    }

    let mut terminated = cmdline.as_bytes().to_vec();
    terminated.push(0);
    memory.write(CMDLINE_ADDRESS, &terminated)?;

    if let Some(initrd) = initrd {
        let end = initrd.address + initrd.size as u64;
        if end > memory.size_bytes() {
            return Err(EngineError::Memory(format!(
                "initramfs ends at {end:#x}, past {:#x} of guest memory",
                memory.size_bytes()
            )));
        }
        header.ramdisk_image = initrd.address as u32;
        header.ramdisk_size = initrd.size as u32;
    }

    // Without a loader identity the kernel treats the initrd as absent: reserve_initrd()
    // returns early on !type_of_loader. 0xFF means "undefined, private use".
    header.type_of_loader = 0xFF;
    // LOADED_HIGH: the kernel is entered in 32-bit protected mode above 1 MiB.
    header.loadflags |= 0x01;
    header.code32_start = kernel_start as u32;
    header.cmd_line_ptr = CMDLINE_ADDRESS as u32;
    header.cmdline_size = terminated.len() as u32;

    memory.write(
        ZERO_PAGE_ADDRESS + SETUP_HEADER_OFFSET,
        &header_field_bytes(&header),
    )?;
    write_e820_map(memory)?;

    Ok(GuestAddress(ZERO_PAGE_ADDRESS))
}

/// Serialise the packed setup header to bytes without taking references to its fields.
fn header_field_bytes(header: &setup_header) -> Vec<u8> {
    let size = std::mem::size_of::<setup_header>();
    let mut bytes = vec![0u8; size];

    // SAFETY: `setup_header` is plain old data with no padding invariants, and both
    // pointers are valid for `size` bytes. Copying bytewise avoids any alignment claim.
    #[allow(unsafe_code)]
    unsafe {
        std::ptr::copy_nonoverlapping(
            (header as *const setup_header).cast::<u8>(),
            bytes.as_mut_ptr(),
            size,
        );
    }

    bytes
}

/// Describe guest RAM to the kernel.
///
/// Without this the decompressor has no memory map, produces no output at all, and the
/// guest appears to hang — the single most deceptive failure in the whole spike.
pub fn write_e820_map(memory: &GuestMemory) -> Result<(), EngineError> {
    let limit = memory.size_bytes();
    let low_end: u64 = 0x0009_F000;
    let high_start: u64 = 0x0010_0000;

    let mut table: Vec<u8> = Vec::new();
    push_e820_record(&mut table, 0, low_end.min(limit), 1);

    if limit > high_start {
        push_e820_record(&mut table, high_start, limit - high_start, 1);
    }

    memory.write(ZERO_PAGE_ADDRESS + E820_TABLE_OFFSET, &table)?;

    let entries = (table.len() / E820_RECORD_SIZE) as u8;
    memory.write(ZERO_PAGE_ADDRESS + E820_ENTRIES_OFFSET, &[entries])?;

    Ok(())
}

fn push_e820_record(table: &mut Vec<u8>, base: u64, length: u64, kind: u32) {
    table.extend_from_slice(&base.to_le_bytes());
    table.extend_from_slice(&length.to_le_bytes());
    table.extend_from_slice(&kind.to_le_bytes());
}

/// Memory a launch needs at minimum, given a kernel's declared workspace.
pub fn required_memory(kernel_workspace: u64, kernel_start: u64) -> MemorySize {
    let needed = kernel_start.saturating_add(kernel_workspace);
    let mib = (needed / (1024 * 1024)) + 64;
    MemorySize::from_mib(mib).unwrap_or(MemorySize::from_mib(64).expect("64 MiB is nonzero"))
}
