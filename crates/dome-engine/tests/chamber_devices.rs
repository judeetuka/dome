//! The device set as a chamber actually gets it.

use dome_engine::devices::virtio_mmio::{VirtioMmio, reg};
use dome_engine::devices::virtio_net::{NetworkBackend, VirtioNet};
use dome_engine::devices::{ChamberIo, Device, VIRTIO_MMIO_BASE};
use dome_engine::vcpu::VcpuIo;

struct Silent;

impl NetworkBackend for Silent {
    fn mac(&self) -> [u8; 6] {
        [0x52, 0x54, 0x00, 0x00, 0x00, 0x01]
    }

    fn transmit(&mut self, _frame: &[u8]) {}

    fn receive(&mut self) -> Option<Vec<u8>> {
        None
    }
}

#[test]
fn a_chamber_gets_a_console_and_a_network_device() {
    let mut io = ChamberIo::with_console_and_network(Silent);

    let windows = io.devices_mut().windows();
    assert_eq!(windows.len(), 2, "console and transport, not one per queue");

    let names: Vec<&str> = windows.iter().map(|(name, _, _)| *name).collect();
    assert!(names.contains(&"uart16550"), "{names:?}");

    let (_, base, _) = windows
        .iter()
        .find(|(name, _, _)| *name != "uart16550")
        .expect("transport");
    assert_eq!(*base, VIRTIO_MMIO_BASE);
}

#[test]
fn the_network_device_answers_the_probe_a_kernel_makes() {
    // A kernel probes by reading the magic value and the version. If either is wrong the device
    // is skipped silently, which is the least debuggable failure there is.
    let mut io = ChamberIo::with_console_and_network(Silent);

    let mut magic = [0u8; 4];
    io.mmio_read(VIRTIO_MMIO_BASE, &mut magic);
    assert_eq!(
        u32::from_le_bytes(magic),
        0x7472_6976,
        "the magic value spells virt in little-endian"
    );

    let mut version = [0u8; 4];
    io.mmio_read(VIRTIO_MMIO_BASE + reg::VERSION, &mut version);
    assert_eq!(u32::from_le_bytes(version), 2);

    let mut device_id = [0u8; 4];
    io.mmio_read(VIRTIO_MMIO_BASE + reg::DEVICE_ID, &mut device_id);
    assert_eq!(
        u32::from_le_bytes(device_id),
        1,
        "device type 1 is a network card"
    );
}

#[test]
fn a_chamber_without_a_network_keeps_the_console_only() {
    let mut io = ChamberIo::with_console();
    assert_eq!(io.devices_mut().windows().len(), 1);
}

#[test]
fn the_transport_can_be_built_directly_for_a_test() {
    // The Windows and macOS engines will construct the same device against their own transport.
    let mut device = VirtioMmio::new(VIRTIO_MMIO_BASE, VirtioNet::new(Silent));
    let mut magic = [0u8; 4];
    Device::mmio_read(&mut device, 0, &mut magic);
    assert_eq!(u32::from_le_bytes(magic), 0x7472_6976);
}
