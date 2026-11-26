//! The device model every engine implements.
//!
//! The set is deliberately tiny, and that is the security boundary rather than an
//! implementation detail: Firecracker's design note makes the same argument, and every device
//! added is another parser the guest can attack. The pinned set is a 16550 UART, a virtio-mmio
//! transport, and the initramfs loader.
//!
//! Nothing here mentions KVM, Hypervisor.framework or WHP. A device is a register file plus a
//! memory view, which is what lets one foundation boot unchanged on all three engines.

pub mod cmos;
pub mod uart16550;
pub mod virtio_mmio;
pub mod virtio_net;

use crate::devices::virtio_net::NetworkBackend;
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

    /// A queue the guest just rang, to be serviced by whoever owns guest memory.
    ///
    /// Devices do not hold a memory view: one notification is the only moment a device needs to
    /// look at RAM, and giving it a permanent reference would mean the engine and the device both
    /// alias the same mapping.
    fn take_doorbell(&mut self) -> Option<u16> {
        None
    }

    /// Service a doorbell with a view of guest memory.
    fn service_doorbell(&mut self, _queue: u16, _memory: &GuestMemory) {}

    /// The virtio-mmio window this device answers on, if it is one.
    ///
    /// Linux on x86 has no device tree, so a virtio-mmio window has to be named on the kernel
    /// command line or it is never probed. That makes this the engine's only way to tell a guest
    /// the device exists at all.
    fn virtio_window(&self) -> Option<(u64, u64)> {
        None
    }

    /// Whether the device is asserting an interrupt the guest has not acknowledged.
    fn interrupt_pending(&self) -> bool {
        false
    }
}

/// The devices attached to one chamber.
#[derive(Default)]
pub struct DeviceSet {
    devices: Vec<Box<dyn Device>>,
    pending: Vec<(usize, u16)>,
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

    /// Service every doorbell rung since the last call.
    pub fn service_pending(&mut self, memory: &GuestMemory) {
        while let Some((index, queue)) = self.pending.pop() {
            if let Some(device) = self.devices.get_mut(index) {
                device.service_doorbell(queue, memory);
            }
        }
    }

    /// Whether a doorbell is waiting to be serviced.
    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    /// The virtio-mmio windows in the set, in address order.
    pub fn virtio_windows(&self) -> Vec<(u64, u64)> {
        let mut windows: Vec<(u64, u64)> = self
            .devices
            .iter()
            .filter_map(|device| device.virtio_window())
            .collect();

        windows.sort();
        windows
    }

    /// Whether any device is asserting an interrupt.
    pub fn interrupt_pending(&self) -> bool {
        self.devices.iter().any(|device| device.interrupt_pending())
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
            // A device that only answers on ports occupies no memory window, and reporting a
            // zero-length one at address zero would put it in the kernel's command line.
            .filter(|(_, _, length)| *length > 0)
            .collect()
    }

    /// Route a guest write, returning whether any device claimed the address.
    pub fn mmio_write(&mut self, address: u64, data: &[u8]) -> bool {
        let Some(index) = self.index_of(address) else {
            return false;
        };

        let offset = address - self.devices[index].mmio_range().0;
        self.devices[index].mmio_write(offset, data);

        // A write may have rung a doorbell. Recording the index rather than a reference keeps the
        // device list borrowable while the engine services it with memory.
        if let Some(queue) = self.devices[index].take_doorbell() {
            self.pending.push((index, queue));
        }

        true
    }

    /// Which device claims an address.
    fn index_of(&self, address: u64) -> Option<usize> {
        self.devices.iter().position(|device| {
            let (base, length) = device.mmio_range();
            address >= base && address < base.saturating_add(length)
        })
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
    /// Whether a port no device owns reads as all-ones, the way an empty bus floats.
    ///
    /// Modern kernels assume real platform hardware (a keyboard controller, an RTC) and spin when
    /// a floated port reads zero, because zero means "device present, nothing to report". Older
    /// test guests do not, so this is a property of the chamber rather than a global rule.
    absent_ports_float_high: bool,
    /// Device accesses, when DOME_ACCESS_STATS is set. A guest that spins looks like a hang; the
    /// addresses it spins on say what it is waiting for.
    access: std::collections::HashMap<String, u64>,
}

impl ChamberIo {
    /// Record one access, when asked to.
    fn note(&mut self, kind: &str, address: u64) {
        if std::env::var_os("DOME_ACCESS_STATS").is_none() {
            return;
        }

        let key = format!("{kind} {address:#x}");
        *self.access.entry(key).or_insert(0) += 1;
    }

    /// The most frequent accesses, for a guest that will not move.
    pub fn hottest_accesses(&self, limit: usize) -> Vec<(String, u64)> {
        let mut all: Vec<(String, u64)> =
            self.access.iter().map(|(k, v)| (k.clone(), *v)).collect();
        all.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
        all.truncate(limit);
        all
    }

    /// A chamber with a console and nothing else, which is the current device set.
    pub fn with_console() -> Self {
        let mut devices = DeviceSet::new();
        devices.add(Box::new(uart16550::Uart16550::new()));
        // The clock is a platform device like the console: a guest assumes it exists, and a
        // floated bus does not read as "absent", it reads as "always busy".
        devices.add(Box::new(cmos::Cmos::new()));
        Self {
            devices,
            access: std::collections::HashMap::new(),
            absent_ports_float_high: false,
        }
    }

    /// A chamber with a console and a network device.
    ///
    /// The network is attached at the standard virtio-mmio window, so the guest finds it the same
    /// way it finds any other virtio device. A chamber built this way is still three-device-shaped
    /// from the engine's point of view: console, transport, and whatever the transport carries.
    pub fn with_console_and_network(backend: impl NetworkBackend + 'static) -> Self {
        let mut devices = DeviceSet::new();
        devices.add(Box::new(uart16550::Uart16550::new()));
        devices.add(Box::new(cmos::Cmos::new()));
        devices.add(Box::new(virtio_mmio::VirtioMmio::new(
            VIRTIO_MMIO_BASE,
            virtio_net::VirtioNet::new(backend),
        )));
        Self {
            devices,
            access: std::collections::HashMap::new(),
            absent_ports_float_high: false,
        }
    }

    /// Make unowned ports read as all-ones, which is what modern kernels expect of a PC.
    pub fn with_floating_bus(mut self) -> Self {
        self.absent_ports_float_high = true;
        self
    }

    /// A chamber with devices the caller chose.
    pub fn from_devices(devices: DeviceSet) -> Self {
        Self {
            devices,
            access: std::collections::HashMap::new(),
            absent_ports_float_high: false,
        }
    }

    /// The device set, for attaching more.
    pub fn devices_mut(&mut self) -> &mut DeviceSet {
        &mut self.devices
    }

    /// Service doorbells rung during the last exit.
    pub fn service_pending(&mut self, memory: &GuestMemory) {
        self.devices.service_pending(memory);
    }

    /// The virtio-mmio windows this chamber exposes.
    pub fn virtio_windows(&self) -> Vec<(u64, u64)> {
        self.devices.virtio_windows()
    }

    /// Whether a device is asserting an interrupt.
    pub fn interrupt_pending(&self) -> bool {
        self.devices.interrupt_pending()
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
        self.note("io-read", u64::from(port));

        if !self.devices.io_read(port, data) && self.absent_ports_float_high {
            // An uninhabited port reads as all-ones, not zero: a real machine has no device there
            // and the bus floats high, where zero says "device present, nothing to report" and a
            // driver waits for a bit that will never change. Measured: 10,003 polls of the
            // keyboard controller on a newer kernel.
            data.fill(0xFF);
        }
    }

    fn io_out(&mut self, port: u16, data: &[u8]) {
        self.note("io-write", u64::from(port));
        self.devices.io_write(port, data);
    }

    fn mmio_read(&mut self, address: u64, data: &mut [u8]) {
        self.note("mmio-read", address);
        self.devices.mmio_read(address, data);
    }

    fn mmio_write(&mut self, address: u64, data: &[u8]) {
        self.note("mmio-write", address);
        self.devices.mmio_write(address, data);
    }
}
