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

    /// The port range this device answers, if it is a legacy port device.
    ///
    /// Only the UART needs this. Modern macOS and Windows engines have no port space at all,
    /// which is why it is optional rather than required.
    fn io_range(&self) -> Option<(u16, u16)> {
        None
    }

    /// Guest read from a port.
    fn io_in(&mut self, _port: u16, data: &mut [u8]) {
        data.fill(0);
    }

    /// Guest write to a port.
    fn io_out(&mut self, _port: u16, _data: &[u8]) {}

    /// Everything the guest has written to this device, if it is a console.
    fn console(&self) -> Option<&[u8]> {
        None
    }
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

    /// Everything the guest has written to the console, whichever path it used.
    pub fn console(&self) -> &[u8] {
        self.devices
            .iter()
            .find_map(|device| device.console())
            .unwrap_or(&[])
    }

    /// Route a guest write to a port, returning whether any device claimed it.
    pub fn io_write(&mut self, port: u16, data: &[u8]) -> bool {
        for device in self.devices.iter_mut() {
            if let Some((base, length)) = device.io_range()
                && port >= base
                && port < base.saturating_add(length)
            {
                device.io_out(port, data);
                return true;
            }
        }

        false
    }

    /// Route a guest read from a port, returning whether any device claimed it.
    pub fn io_read(&mut self, port: u16, data: &mut [u8]) -> bool {
        for device in self.devices.iter_mut() {
            if let Some((base, length)) = device.io_range()
                && port >= base
                && port < base.saturating_add(length)
            {
                device.io_in(port, data);
                return true;
            }
        }

        data.fill(0);
        false
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

/// The devices a running chamber exposes, behind the engine's IO trait.
///
/// One object serves both paths: a legacy port write from a BIOS-era guest and a memory-mapped
/// write from a modern one reach the same device, so the two cannot disagree about state.
pub struct ChamberIo {
    devices: DeviceSet,
}

impl ChamberIo {
    /// A chamber with a console and nothing else, which is the current device set.
    pub fn with_console() -> Self {
        let mut devices = DeviceSet::new();
        devices.add(Box::new(uart16550::Uart16550::new()));
        Self { devices }
    }

    /// A chamber with devices the caller chose.
    pub fn from_devices(devices: DeviceSet) -> Self {
        Self { devices }
    }

    /// The device set, for attaching more.
    pub fn devices_mut(&mut self) -> &mut DeviceSet {
        &mut self.devices
    }

    /// Everything the guest has written to the console.
    pub fn console(&self) -> &[u8] {
        self.devices.console()
    }

    /// The console as text, lossily.
    pub fn console_text(&self) -> String {
        String::from_utf8_lossy(self.console()).into_owned()
    }
}

impl crate::vcpu::VcpuIo for ChamberIo {
    fn io_in(&mut self, port: u16, data: &mut [u8]) {
        self.devices.io_read(port, data);
    }

    fn io_out(&mut self, port: u16, data: &[u8]) {
        self.devices.io_write(port, data);
    }

    fn mmio_read(&mut self, address: u64, data: &mut [u8]) {
        self.devices.mmio_read(address, data);
    }

    fn mmio_write(&mut self, address: u64, data: &[u8]) {
        self.devices.mmio_write(address, data);
    }
}
