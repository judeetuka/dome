//! The virtio-mmio transport.
//!
//! Deliberately not PCI: the MMIO transport is a flat register window with no configuration
//! space, no BARs and no bus scanning, which is exactly why Firecracker and every other
//! microVM picked it. Fewer parsers the guest can attack.
//!
//! This is the transport only. Device behaviour lives behind [VirtioBackend], so a block device
//! can arrive later without touching the handshake.

use crate::devices::Device;

/// Magic value a driver reads to confirm it found virtio.
pub const MAGIC: u32 = 0x7472_6976;
/// Transport version. Two is the modern (non-legacy) layout.
pub const VERSION: u32 = 2;

/// Register offsets, from the virtio specification section 4.2.2.
pub mod reg {
    /// Device identity.
    pub const MAGIC_VALUE: u64 = 0x000;
    /// Transport version.
    pub const VERSION: u64 = 0x004;
    /// Device type.
    pub const DEVICE_ID: u64 = 0x008;
    /// Vendor identity.
    pub const VENDOR_ID: u64 = 0x00c;
    /// Low word of the device feature bits.
    pub const DEVICE_FEATURES: u64 = 0x010;
    /// Which feature word the next read or write refers to.
    pub const DEVICE_FEATURES_SEL: u64 = 0x014;
    /// Low word of the features the driver accepted.
    pub const DRIVER_FEATURES: u64 = 0x020;
    /// Which driver feature word the next write refers to.
    pub const DRIVER_FEATURES_SEL: u64 = 0x024;
    /// Which queue the following queue registers address.
    pub const QUEUE_SEL: u64 = 0x030;
    /// Largest queue this device accepts.
    pub const QUEUE_NUM_MAX: u64 = 0x034;
    /// Number of descriptors the driver chose.
    pub const QUEUE_NUM: u64 = 0x038;
    /// Whether the driver has finished setting the queue up.
    pub const QUEUE_READY: u64 = 0x044;
    /// Ringing the doorbell.
    pub const QUEUE_NOTIFY: u64 = 0x050;
    /// Interrupt bits the device is asserting.
    pub const INTERRUPT_STATUS: u64 = 0x060;
    /// Acknowledge and clear interrupt bits.
    pub const INTERRUPT_ACK: u64 = 0x064;
    /// Device status.
    pub const STATUS: u64 = 0x070;
    /// Low word of the descriptor table address.
    pub const QUEUE_DESC_LOW: u64 = 0x080;
    /// High word of the descriptor table address.
    pub const QUEUE_DESC_HIGH: u64 = 0x084;
    /// Low word of the driver's available ring address.
    pub const QUEUE_DRIVER_LOW: u64 = 0x090;
    /// High word of the driver's available ring address.
    pub const QUEUE_DRIVER_HIGH: u64 = 0x094;
    /// Low word of the device's used ring address.
    pub const QUEUE_DEVICE_LOW: u64 = 0x0a0;
    /// High word of the device's used ring address.
    pub const QUEUE_DEVICE_HIGH: u64 = 0x0a4;
    /// Configuration generation, so a driver can detect changes.
    pub const CONFIG_GENERATION: u64 = 0x0fc;
    /// Start of device-specific configuration.
    pub const CONFIG: u64 = 0x100;
}

/// Device status bits, from the specification.
pub mod status {
    /// The driver has found the device.
    pub const ACKNOWLEDGE: u8 = 1;
    /// The driver knows how to drive it.
    pub const DRIVER: u8 = 2;
    /// The driver is ready for the device to run.
    pub const DRIVER_OK: u8 = 4;
    /// The driver has negotiated features.
    pub const FEATURES_OK: u8 = 8;
    /// The device has given up.
    pub const FAILED: u8 = 0x80;
}

/// Interrupt bits.
pub mod interrupt {
    /// The used ring has been updated.
    pub const USED_BUFFER: u8 = 1;
}

/// Largest queue this transport accepts.
pub const QUEUE_NUM_MAX: u32 = 256;

/// One entry of the descriptor table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Descriptor {
    /// Guest physical address of the buffer.
    pub address: u64,
    /// Length of the buffer.
    pub length: u32,
    /// Next / write-only flags.
    pub flags: u16,
    /// The next descriptor in the chain.
    pub next: u16,
}

/// Descriptor flags.
pub mod descriptor_flags {
    /// The buffer continues in `next`.
    pub const NEXT: u16 = 1;
    /// The driver may only write to this buffer.
    pub const WRITE: u16 = 2;
}

/// What a device type does once the transport hands it a queue.
pub trait VirtioBackend {
    /// The virtio device type, for example 2 for a block device.
    fn device_id(&self) -> u32;

    /// The feature bits this device offers.
    fn device_features(&self) -> u64;

    /// Read device-specific configuration.
    fn read_config(&self, offset: u64, data: &mut [u8]);

    /// Write device-specific configuration.
    fn write_config(&mut self, offset: u64, data: &[u8]);

    /// The driver rang the doorbell for a queue. Returns whether an interrupt is owed.
    ///
    /// Memory is handed over rather than stored because a device never owns the guest's RAM: it
    /// borrows a view for the duration of one notification. That is also why the transport cannot
    /// service a doorbell itself - it does not have the memory, the engine does.
    fn notify(
        &mut self,
        queue: u16,
        state: QueueState,
        memory: &crate::memory::GuestMemory,
    ) -> bool;
}

/// State of one queue as the driver configured it.
#[derive(Debug, Clone, Copy, Default)]
pub struct QueueState {
    /// Descriptors the driver asked for.
    pub num: u16,
    /// Whether the driver declared the queue ready.
    pub ready: bool,
    /// Descriptor table address.
    pub descriptor_table: u64,
    /// Available ring address.
    pub available: u64,
    /// Used ring address.
    pub used: u64,
}

/// The transport.
pub struct VirtioMmio<B: VirtioBackend> {
    base: u64,
    backend: B,
    status: u8,
    device_features_sel: u32,
    driver_features_sel: u32,
    driver_features: u64,
    queue_sel: u16,
    queues: Vec<QueueState>,
    interrupt_status: u8,
    pending_doorbell: Option<u16>,
}

impl<B: VirtioBackend> VirtioMmio<B> {
    /// Attach a backend at a base address, with room for four queues.
    pub fn new(base: u64, backend: B) -> Self {
        Self {
            base,
            backend,
            status: 0,
            device_features_sel: 0,
            driver_features_sel: 0,
            driver_features: 0,
            queue_sel: 0,
            queues: vec![QueueState::default(); 4],
            interrupt_status: 0,
            pending_doorbell: None,
        }
    }

    /// The backend, for inspecting what the driver did.
    pub fn backend(&self) -> &B {
        &self.backend
    }

    /// The backend, mutably.
    pub fn backend_mut(&mut self) -> &mut B {
        &mut self.backend
    }

    /// The device status the driver has negotiated.
    pub fn status(&self) -> u8 {
        self.status
    }

    /// Whether the driver has finished and the device is live.
    pub fn is_driving(&self) -> bool {
        self.status & status::DRIVER_OK != 0 && self.status & status::FAILED == 0
    }

    /// Whether the device gave up, which happens when a driver asks for unsupported features.
    pub fn has_failed(&self) -> bool {
        self.status & status::FAILED != 0
    }

    /// The features the driver accepted.
    pub fn driver_features(&self) -> u64 {
        self.driver_features
    }

    /// A queue's configuration.
    pub fn queue(&self, index: u16) -> Option<QueueState> {
        self.queues.get(usize::from(index)).copied()
    }

    /// Which queue the driver last selected.
    pub fn selected_queue(&self) -> u16 {
        self.queue_sel
    }

    /// Read a 32-bit register.
    pub fn read_u32(&mut self, offset: u64) -> u32 {
        match offset {
            reg::MAGIC_VALUE => MAGIC,
            reg::VERSION => VERSION,
            reg::DEVICE_ID => self.backend.device_id(),
            reg::VENDOR_ID => 0,
            reg::DEVICE_FEATURES => {
                let features = self.backend.device_features();
                if self.device_features_sel == 0 {
                    features as u32
                } else {
                    (features >> 32) as u32
                }
            }
            reg::QUEUE_NUM_MAX => QUEUE_NUM_MAX,
            reg::QUEUE_NUM => u32::from(self.current_queue().copied().unwrap_or_default().num),
            reg::QUEUE_READY => u32::from(u8::from(
                self.current_queue().map(|q| q.ready).unwrap_or(false),
            )),
            reg::INTERRUPT_STATUS => u32::from(self.interrupt_status),
            reg::STATUS => u32::from(self.status),
            reg::CONFIG_GENERATION => 0,
            reg::QUEUE_DESC_LOW => self
                .current_queue()
                .map(|q| q.descriptor_table as u32)
                .unwrap_or(0),
            reg::QUEUE_DESC_HIGH => self
                .current_queue()
                .map(|q| (q.descriptor_table >> 32) as u32)
                .unwrap_or(0),
            reg::QUEUE_DRIVER_LOW => self
                .current_queue()
                .map(|q| q.available as u32)
                .unwrap_or(0),
            reg::QUEUE_DRIVER_HIGH => self
                .current_queue()
                .map(|q| (q.available >> 32) as u32)
                .unwrap_or(0),
            reg::QUEUE_DEVICE_LOW => self.current_queue().map(|q| q.used as u32).unwrap_or(0),
            reg::QUEUE_DEVICE_HIGH => self
                .current_queue()
                .map(|q| (q.used >> 32) as u32)
                .unwrap_or(0),
            _ => 0,
        }
    }

    /// Write a 32-bit register.
    pub fn write_u32(&mut self, offset: u64, value: u32) {
        match offset {
            reg::DEVICE_FEATURES_SEL => self.device_features_sel = value,
            reg::DRIVER_FEATURES_SEL => self.driver_features_sel = value,
            reg::DRIVER_FEATURES => {
                let offered = self.backend.device_features();
                let shift = if self.driver_features_sel == 0 { 0 } else { 32 };
                let wide = u64::from(value) << shift;

                // Only bits the device offered may be accepted. Asking for anything else means
                // the driver does not understand this device, and the specification says the
                // device must refuse rather than guess.
                if wide & !offered != 0 {
                    self.status |= status::FAILED;
                    return;
                }

                let slot = u64::from(u32::MAX) << shift;
                self.driver_features = (self.driver_features & !slot) | wide;
            }
            reg::QUEUE_SEL => self.queue_sel = value as u16,
            reg::QUEUE_NUM => {
                let requested = value as u16;
                if let Some(queue) = self.current_queue() {
                    // A driver asking for too many descriptors, or none, gets the maximum it
                    // is allowed rather than a silently different queue size.
                    queue.num = if requested == 0 || u32::from(requested) > QUEUE_NUM_MAX {
                        QUEUE_NUM_MAX as u16
                    } else {
                        requested
                    };
                }
            }
            reg::QUEUE_READY => {
                if let Some(queue) = self.current_queue() {
                    queue.ready = value != 0;
                }
            }
            reg::QUEUE_NOTIFY => {
                // Deferred: the backend needs guest memory to do anything with a queue, and the
                // transport does not have it. The engine services this after the exit.
                self.pending_doorbell = Some(value as u16);
            }
            reg::INTERRUPT_ACK => self.interrupt_status &= !(value as u8),
            reg::STATUS => self.write_status(value as u8),
            reg::QUEUE_DESC_LOW => self.set_queue_field(value.into(), QueueField::DescriptorLow),
            reg::QUEUE_DESC_HIGH => {
                self.set_queue_field(u64::from(value) << 32, QueueField::DescriptorHigh)
            }
            reg::QUEUE_DRIVER_LOW => self.set_queue_field(value.into(), QueueField::DriverLow),
            reg::QUEUE_DRIVER_HIGH => {
                self.set_queue_field(u64::from(value) << 32, QueueField::DriverHigh)
            }
            reg::QUEUE_DEVICE_LOW => self.set_queue_field(value.into(), QueueField::DeviceLow),
            reg::QUEUE_DEVICE_HIGH => {
                self.set_queue_field(u64::from(value) << 32, QueueField::DeviceHigh)
            }
            _ => {}
        }
    }

    fn write_status(&mut self, value: u8) {
        // Writing zero resets the device, per the specification.
        if value == 0 {
            self.status = 0;
            self.driver_features = 0;
            self.interrupt_status = 0;
            self.queue_sel = 0;
            for queue in &mut self.queues {
                *queue = QueueState::default();
            }
            return;
        }

        self.status = value;

        // A driver that declares FEATURES_OK is asking the device to confirm the feature set.
        // The device confirms by leaving the bit set; anything it cannot support was already
        // refused when the driver wrote the feature words.
        if value & status::FEATURES_OK != 0
            && self.driver_features & !self.backend.device_features() != 0
        {
            self.status &= !status::FEATURES_OK;
            self.status |= status::FAILED;
        }
    }

    fn current_queue(&mut self) -> Option<&mut QueueState> {
        self.queues.get_mut(usize::from(self.queue_sel))
    }

    fn set_queue_field(&mut self, value: u64, field: QueueField) {
        let Some(queue) = self.current_queue() else {
            return;
        };

        match field {
            QueueField::DescriptorLow => {
                queue.descriptor_table = (queue.descriptor_table & !0xffff_ffff) | value
            }
            QueueField::DescriptorHigh => {
                queue.descriptor_table = (queue.descriptor_table & 0xffff_ffff) | value
            }
            QueueField::DriverLow => queue.available = (queue.available & !0xffff_ffff) | value,
            QueueField::DriverHigh => queue.available = (queue.available & 0xffff_ffff) | value,
            QueueField::DeviceLow => queue.used = (queue.used & !0xffff_ffff) | value,
            QueueField::DeviceHigh => queue.used = (queue.used & 0xffff_ffff) | value,
        }
    }
}

enum QueueField {
    DescriptorLow,
    DescriptorHigh,
    DriverLow,
    DriverHigh,
    DeviceLow,
    DeviceHigh,
}

impl<B: VirtioBackend + 'static> Device for VirtioMmio<B> {
    fn name(&self) -> &'static str {
        "virtio-mmio"
    }

    fn mmio_range(&self) -> (u64, u64) {
        (self.base, crate::devices::VIRTIO_MMIO_SIZE)
    }

    fn mmio_read(&mut self, offset: u64, data: &mut [u8]) {
        if offset >= reg::CONFIG {
            self.backend.read_config(offset - reg::CONFIG, data);
            return;
        }

        let value = self.read_u32(offset).to_le_bytes();
        copy(value.as_slice(), data);
    }

    fn mmio_write(&mut self, offset: u64, data: &[u8]) {
        if offset >= reg::CONFIG {
            self.backend.write_config(offset - reg::CONFIG, data);
            return;
        }

        let mut value = [0u8; 4];
        copy_into(data, &mut value);
        self.write_u32(offset, u32::from_le_bytes(value));
    }
    fn take_doorbell(&mut self) -> Option<u16> {
        self.pending_doorbell.take()
    }

    fn virtio_window(&self) -> Option<(u64, u64)> {
        Some((self.base, crate::devices::VIRTIO_MMIO_SIZE))
    }

    fn interrupt_pending(&self) -> bool {
        self.interrupt_status != 0
    }

    fn service_doorbell(&mut self, queue: u16, memory: &crate::memory::GuestMemory) {
        let state = self
            .queues
            .get(usize::from(queue))
            .copied()
            .unwrap_or_default();

        if !state.ready {
            return;
        }

        if self.backend.notify(queue, state, memory) {
            self.interrupt_status |= interrupt::USED_BUFFER;
        }
    }
}

fn copy(source: &[u8], destination: &mut [u8]) {
    let length = source.len().min(destination.len());
    destination[..length].copy_from_slice(&source[..length]);
    for byte in destination.iter_mut().skip(length) {
        *byte = 0;
    }
}

fn copy_into(source: &[u8], destination: &mut [u8]) {
    let length = source.len().min(destination.len());
    destination[..length].copy_from_slice(&source[..length]);
}
