//! The device model every engine implements.
//!
//! The set is deliberately tiny, and that is the security boundary rather than an
//! implementation detail: Firecracker's design note makes the same argument, and every device
//! added is another parser the guest can attack. The pinned set is a 16550 UART, a virtio-mmio
//! transport, and the initramfs loader.
//!
//! Nothing here mentions KVM, Hypervisor.framework or WHP. A device is a register file plus a
//! memory view, which is what lets one foundation boot unchanged on all three engines.

pub mod uart16550;
pub mod virtio_mmio;

use crate::memory::GuestMemory;

/// One device the guest can reach.
///
/// Addresses are offsets from the device's own base, so a device does not need to know where
/// the engine decided to put it.
pub trait Device {
    /// A short name, for reports and diagnostics.
    fn name(&self) -> &'static str;

    /// The memory-mapped window this device occupies.
    fn mmio_range(&self) -> (u64, u64);

    /// Guest read from the window.
    fn mmio_read(&mut self, offset: u64, data: &mut [u8]);

    /// Guest write to the window.
    fn mmio_write(&mut self, offset: u64, data: &[u8]);
}

/// The devices attached to one chamber.
#[derive(Default)]
pub struct DeviceSet {
    devices: Vec<Box<dyn Device>>,
}

impl DeviceSet {
    /// An empty set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Attach a device.
    pub fn add(&mut self, device: Box<dyn Device>) {
        self.devices.push(device);
    }

    /// How many devices are attached.
    pub fn len(&self) -> usize {
        self.devices.len()
    }

    /// Whether the set is empty.
    pub fn is_empty(&self) -> bool {
        self.devices.is_empty()
    }

    /// The window each device occupies.
    pub fn windows(&self) -> Vec<(&'static str, u64, u64)> {
        self.devices
            .iter()
            .map(|device| {
                let (base, length) = device.mmio_range();
                (device.name(), base, length)
            })
            .collect()
    }

    /// Route a guest write, returning whether any device claimed the address.
    pub fn mmio_write(&mut self, address: u64, data: &[u8]) -> bool {
        let Some((device, offset)) = self.route(address) else {
            return false;
        };

        device.mmio_write(offset, data);
        true
    }

    /// Route a guest read, returning whether any device claimed the address.
    pub fn mmio_read(&mut self, address: u64, data: &mut [u8]) -> bool {
        let Some((device, offset)) = self.route(address) else {
            data.fill(0);
            return false;
        };

        device.mmio_read(offset, data);
        true
    }

    fn route(&mut self, address: u64) -> Option<(&mut Box<dyn Device>, u64)> {
        for device in self.devices.iter_mut() {
            let (base, length) = device.mmio_range();
            if address >= base && address < base.saturating_add(length) {
                return Some((device, address - base));
            }
        }

        None
    }
}

/// Address of the first virtio-mmio window. The kernel looks here when told to probe.
pub const VIRTIO_MMIO_BASE: u64 = 0xd000_0000;
/// Size of one virtio-mmio window.
pub const VIRTIO_MMIO_SIZE: u64 = 0x1000;
/// How many virtio-mmio windows an engine may expose.
pub const VIRTIO_MMIO_COUNT: u64 = 8;

/// Read the virtio descriptor table entry at `index`.
///
/// Split out from the transport because it is the part that has to agree with the specification
/// exactly, and it is far easier to test on its own.
pub fn read_descriptor(
    memory: &GuestMemory,
    table: u64,
    index: u16,
) -> Result<virtio_mmio::Descriptor, crate::EngineError> {
    let offset = table + u64::from(index) * 16;
    let bytes = memory.read(offset, 16)?;

    Ok(virtio_mmio::Descriptor {
        address: u64::from_le_bytes(bytes[0..8].try_into().unwrap_or([0; 8])),
        length: u32::from_le_bytes(bytes[8..12].try_into().unwrap_or([0; 4])),
        flags: u16::from_le_bytes(bytes[12..14].try_into().unwrap_or([0; 2])),
        next: u16::from_le_bytes(bytes[14..16].try_into().unwrap_or([0; 2])),
    })
}
