//! The device model, tested without a hypervisor.
//!
//! Every device is a register file plus a memory view, so none of this needs KVM. That is
//! deliberate: the transport handshake is the part most likely to be subtly wrong, and it should
//! be provable on a machine with no virtualisation at all.

use dome_core::MemorySize;
use dome_engine::EngineError;
use dome_engine::devices::uart16550::{SERIAL_MMIO_BASE, Uart16550};
use dome_engine::devices::virtio_mmio::{
    Descriptor, MAGIC, QUEUE_NUM_MAX, VERSION, VirtioBackend, VirtioMmio, reg, status,
};
use dome_engine::devices::{ChamberIo, Device, DeviceSet, VIRTIO_MMIO_BASE, read_descriptor};
use dome_engine::memory::GuestMemory;
use dome_engine::vcpu::VcpuIo;

struct FakeBackend {
    notifies: Vec<u16>,
    config: Vec<u8>,
    features: u64,
}

impl FakeBackend {
    fn new() -> Self {
        Self {
            notifies: Vec::new(),
            config: vec![0u8; 8],
            features: 0b1010,
        }
    }
}

impl VirtioBackend for FakeBackend {
    fn device_id(&self) -> u32 {
        2
    }

    fn device_features(&self) -> u64 {
        self.features
    }

    fn read_config(&self, offset: u64, data: &mut [u8]) {
        for (index, byte) in data.iter_mut().enumerate() {
            *byte = self
                .config
                .get(offset as usize + index)
                .copied()
                .unwrap_or(0);
        }
    }

    fn write_config(&mut self, offset: u64, data: &[u8]) {
        for (index, byte) in data.iter().enumerate() {
            if let Some(slot) = self.config.get_mut(offset as usize + index) {
                *slot = *byte;
            }
        }
    }

    fn notify(&mut self, queue: u16) -> bool {
        self.notifies.push(queue);
        true
    }
}

// --- the console ---

#[test]
fn the_uart_captures_what_the_guest_writes() {
    let mut uart = Uart16550::new();

    for byte in b"hello\n" {
        uart.write_port(0, *byte);
    }

    assert_eq!(uart.text(), "hello\n");
}

#[test]
fn the_uart_reports_transmitter_ready() {
    let uart = Uart16550::new();

    // A guest that polls this and never sees readiness silently drops every character.
    assert_eq!(
        uart.read_port(0x05) & 0x60,
        0x60,
        "line status must say ready"
    );
    assert_eq!(uart.read_port(0x02), 0x01, "no interrupt pending");
    assert_eq!(uart.read_port(0x06), 0xB0, "modem status asserted");
}

#[test]
fn the_port_and_memory_mapped_paths_agree() {
    let mut port = Uart16550::new();
    let mut mmio = Uart16550::new();

    port.write_port(0, b'x');
    mmio.write_mmio(0, b'x');

    assert_eq!(port.output(), mmio.output());
    assert_eq!(port.read_port(0x05), mmio.read_mmio(0x05));
}

// --- routing ---

#[test]
fn the_device_set_routes_by_window() {
    let mut set = DeviceSet::new();
    set.add(Box::new(Uart16550::new()));
    set.add(Box::new(VirtioMmio::new(
        VIRTIO_MMIO_BASE,
        FakeBackend::new(),
    )));

    assert_eq!(set.len(), 2);
    let windows = set.windows();
    assert!(windows.iter().any(|(name, _, _)| *name == "uart16550"));
    assert!(windows.iter().any(|(name, _, _)| *name == "virtio-mmio"));

    // A write inside the UART window is claimed.
    assert!(set.mmio_write(SERIAL_MMIO_BASE, b"A"));

    // One outside every window is not, and a read there is zeroed rather than garbage.
    assert!(!set.mmio_write(0xdead_0000, b"B"));
    let mut buffer = [0xFFu8; 4];
    assert!(!set.mmio_read(0xdead_0000, &mut buffer));
    assert_eq!(buffer, [0; 4]);
}

// --- the transport ---

fn transport() -> VirtioMmio<FakeBackend> {
    VirtioMmio::new(VIRTIO_MMIO_BASE, FakeBackend::new())
}

#[test]
fn the_transport_identifies_itself() {
    let mut device = transport();

    assert_eq!(device.read_u32(reg::MAGIC_VALUE), MAGIC);
    assert_eq!(device.read_u32(reg::VERSION), VERSION);
    assert_eq!(device.read_u32(reg::DEVICE_ID), 2, "a block device");
}

#[test]
fn the_status_handshake_reaches_driving() {
    let mut device = transport();
    assert!(!device.is_driving());

    device.write_u32(reg::STATUS, u32::from(status::ACKNOWLEDGE));
    device.write_u32(reg::STATUS, u32::from(status::ACKNOWLEDGE | status::DRIVER));
    device.write_u32(
        reg::STATUS,
        u32::from(status::ACKNOWLEDGE | status::DRIVER | status::FEATURES_OK),
    );
    device.write_u32(
        reg::STATUS,
        u32::from(status::ACKNOWLEDGE | status::DRIVER | status::FEATURES_OK | status::DRIVER_OK),
    );

    assert!(device.is_driving());
    assert!(!device.has_failed());
}

#[test]
fn writing_zero_resets_the_device() {
    let mut device = transport();
    device.write_u32(
        reg::STATUS,
        u32::from(status::ACKNOWLEDGE | status::DRIVER | status::DRIVER_OK),
    );
    device.write_u32(reg::QUEUE_SEL, 0);
    device.write_u32(reg::QUEUE_NUM, 8);

    device.write_u32(reg::STATUS, 0);

    assert_eq!(device.status(), 0);
    assert!(!device.is_driving());
    assert_eq!(
        device.queue(0).map(|queue| queue.num),
        Some(0),
        "the queue was cleared"
    );
}

#[test]
fn the_driver_may_only_accept_offered_features() {
    let mut device = transport();

    // The backend offers 0b1010.
    device.write_u32(reg::DRIVER_FEATURES_SEL, 0);
    device.write_u32(reg::DRIVER_FEATURES, 0b1010);
    assert_eq!(device.driver_features(), 0b1010);
    assert!(!device.has_failed());

    // Asking for a bit that was never offered means the driver does not understand this
    // device, and the specification says to refuse rather than guess.
    device.write_u32(reg::DRIVER_FEATURES, 0b1_0000);
    assert!(
        device.has_failed(),
        "an unoffered feature must fail the device"
    );
}

#[test]
fn the_upper_feature_word_is_addressed_separately() {
    let mut device = transport();
    device.backend_mut().features = 1u64 << 40;

    device.write_u32(reg::DRIVER_FEATURES_SEL, 1);
    device.write_u32(reg::DRIVER_FEATURES, 1u32 << 8); // bit 40 overall

    assert_eq!(device.driver_features(), 1u64 << 40);
    assert!(!device.has_failed());
}

#[test]
fn the_queue_size_is_clamped_to_what_the_device_accepts() {
    let mut device = transport();

    assert_eq!(device.read_u32(reg::QUEUE_NUM_MAX), QUEUE_NUM_MAX);

    device.write_u32(reg::QUEUE_SEL, 0);
    device.write_u32(reg::QUEUE_NUM, 4);
    assert_eq!(device.queue(0).map(|queue| queue.num), Some(4));

    // Asking for more than the device accepts must not silently produce a bigger queue.
    device.write_u32(reg::QUEUE_NUM, 4096);
    assert_eq!(
        device.queue(0).map(|queue| queue.num),
        Some(QUEUE_NUM_MAX as u16)
    );
}

#[test]
fn queue_addresses_are_assembled_from_low_and_high_words() {
    let mut device = transport();
    device.write_u32(reg::QUEUE_SEL, 1);

    device.write_u32(reg::QUEUE_DESC_LOW, 0x1000);
    device.write_u32(reg::QUEUE_DESC_HIGH, 0x2);
    device.write_u32(reg::QUEUE_DRIVER_LOW, 0x2000);
    device.write_u32(reg::QUEUE_DEVICE_LOW, 0x3000);
    device.write_u32(reg::QUEUE_READY, 1);

    let queue = device.queue(1).expect("queue 1");
    assert_eq!(queue.descriptor_table, 0x0000_0002_0000_1000);
    assert_eq!(queue.available, 0x2000);
    assert_eq!(queue.used, 0x3000);
    assert!(queue.ready);

    // Queue 0 was never touched.
    assert_eq!(device.queue(0).map(|queue| queue.ready), Some(false));
}

#[test]
fn a_doorbell_reaches_the_backend_and_raises_an_interrupt() {
    let mut device = transport();

    device.write_u32(reg::QUEUE_NOTIFY, 1);

    assert_eq!(device.backend().notifies, vec![1]);
    assert_eq!(
        device.read_u32(reg::INTERRUPT_STATUS),
        1,
        "an interrupt is owed"
    );

    device.write_u32(reg::INTERRUPT_ACK, 1);
    assert_eq!(
        device.read_u32(reg::INTERRUPT_STATUS),
        0,
        "acknowledging clears it"
    );
}

#[test]
fn device_configuration_is_routed_to_the_backend() {
    let mut device = transport();

    device.mmio_write(reg::CONFIG + 2, &[0xAB]);
    let mut byte = [0u8; 1];
    device.mmio_read(reg::CONFIG + 2, &mut byte);

    assert_eq!(byte, [0xAB]);
}

#[test]
fn a_register_read_through_the_window_matches_the_direct_read() {
    let mut device = transport();
    let mut bytes = [0u8; 4];

    device.mmio_read(reg::MAGIC_VALUE, &mut bytes);

    assert_eq!(u32::from_le_bytes(bytes), MAGIC);
}

// --- descriptor tables ---

#[test]
fn descriptors_are_read_from_guest_memory() {
    let memory = GuestMemory::allocate(MemorySize::from_mib(4).expect("4 MiB")).expect("memory");
    let table = 0x1000u64;

    let expected = Descriptor {
        address: 0x8000,
        length: 512,
        flags: 2,
        next: 0,
    };

    let mut bytes = Vec::new();
    bytes.extend_from_slice(&expected.address.to_le_bytes());
    bytes.extend_from_slice(&expected.length.to_le_bytes());
    bytes.extend_from_slice(&expected.flags.to_le_bytes());
    bytes.extend_from_slice(&expected.next.to_le_bytes());
    memory.write(table, &bytes).expect("write descriptor");

    let read = read_descriptor(&memory, table, 0).expect("read");
    assert_eq!(read, expected);

    // The second entry is further along the table.
    memory.write(table + 16, &bytes).expect("write second");
    assert_eq!(read_descriptor(&memory, table, 1).expect("read"), expected);
}

#[test]
fn reading_a_descriptor_past_the_end_of_guest_memory_is_an_error() {
    let memory = GuestMemory::allocate(MemorySize::from_mib(4).expect("4 MiB")).expect("memory");

    let result = read_descriptor(&memory, 0xffff_ffff, 0);
    assert!(
        matches!(
            result,
            Err(EngineError::Memory(_)) | Err(EngineError::Setup(_))
        ),
        "{result:?}"
    );
}

// --- one io object for the run loop ---

#[test]
fn the_chamber_io_serves_both_the_port_and_mmio_paths() {
    let mut io = ChamberIo::with_console();

    // A BIOS-era guest writes to the port.
    io.io_out(0x3F8, b"A");

    // A modern guest writes to the device window.
    io.mmio_write(SERIAL_MMIO_BASE, b"B");

    // Both reach the same console, in the order they happened.
    assert_eq!(io.console_text(), "AB");
}

#[test]
fn the_chamber_io_ignores_a_port_no_device_owns() {
    let mut io = ChamberIo::with_console();

    io.io_out(0x9999, b"x");
    assert_eq!(io.console_text(), "", "nothing should have been captured");

    // A read from an unowned port is zeroed rather than left as garbage.
    let mut data = [0xFFu8; 2];
    io.io_in(0x9999, &mut data);
    assert_eq!(data, [0, 0]);
}

#[test]
fn the_chamber_io_answers_status_reads_over_the_port_path() {
    let mut io = ChamberIo::with_console();
    let mut line_status = [0u8; 1];

    io.io_in(0x3FD, &mut line_status);

    assert_eq!(line_status[0] & 0x60, 0x60, "transmitter must report ready");
}

#[test]
fn the_chamber_io_ignores_memory_no_device_owns() {
    let mut io = ChamberIo::with_console();

    // Writing into unmapped space must be harmless rather than a panic.
    io.mmio_write(0xdead_beef, b"x");
    assert_eq!(io.console_text(), "");
}
