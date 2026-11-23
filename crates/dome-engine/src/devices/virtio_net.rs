//! The network device.
//!
//! A chamber that installs its own packages has to reach the network, and the honest way to give
//! it one is a virtio device rather than a host proxy: the guest drives its own stack, and the
//! engine sees frames, not intentions.
//!
//! Two queues, as the specification lays them out: queue 0 is receive, queue 1 is transmit. The
//! device is deliberately without the offload features. A guest that computes its own checksums is
//! a guest whose frames can be verified, and every offload not offered is a parser not written.

use crate::devices::virtio_mmio::{Descriptor, QueueState, VirtioBackend, descriptor_flags};
use crate::memory::GuestMemory;
use vm_memory::{Bytes, GuestAddress};

/// The virtio device type for a network card.
pub const DEVICE_ID: u32 = 1;

/// Queue 0 receives frames from the network.
pub const RX_QUEUE: u16 = 0;
/// Queue 1 transmits frames to the network.
pub const TX_QUEUE: u16 = 1;

/// Feature bits this device offers.
pub mod features {
    /// The device announces a MAC address in its configuration space.
    pub const MAC: u64 = 1 << 5;
    /// The device can report link status.
    pub const STATUS: u64 = 1 << 16;
}

/// Size of one descriptor record in the table.
const DESCRIPTOR_SIZE: u16 = 16;

/// What a network backend does with frames.
///
/// The engine does not interpret frames; it moves them. That keeps the packet code in one place
/// and lets a test backend hand the device bytes without a network at all.
pub trait NetworkBackend {
    /// The MAC address the device announces.
    fn mac(&self) -> [u8; 6];

    /// A frame the guest sent.
    fn transmit(&mut self, frame: &[u8]);

    /// A frame to deliver to the guest, if there is one.
    fn receive(&mut self) -> Option<Vec<u8>>;

    /// Whether the link is up.
    fn link_up(&self) -> bool {
        true
    }
}

/// A virtio network device.
pub struct VirtioNet<B: NetworkBackend> {
    backend: B,
    rx_last_available: u16,
    tx_last_available: u16,
    rx_used: u16,
    tx_used: u16,
}

impl<B: NetworkBackend> VirtioNet<B> {
    /// A device driving the given backend.
    pub fn new(backend: B) -> Self {
        Self {
            backend,
            rx_last_available: 0,
            tx_last_available: 0,
            rx_used: 0,
            tx_used: 0,
        }
    }

    /// The backend, for tests and diagnostics.
    pub fn backend(&self) -> &B {
        &self.backend
    }

    /// The backend, mutably.
    pub fn backend_mut(&mut self) -> &mut B {
        &mut self.backend
    }

    /// Frames the guest handed over since the last notification.
    fn drain_transmit(&mut self, state: QueueState, memory: &GuestMemory) {
        let (available_index, entries) = available_entries(memory, state, self.tx_last_available);
        self.tx_last_available = available_index;

        for head in entries {
            let Some(frame) = read_chain(memory, state, head) else {
                // A descriptor chain we cannot read is a chain we must not guess at.
                continue;
            };

            self.backend.transmit(&frame);

            self.tx_used = push_used(memory, state, self.tx_used, head, 0);
        }
    }

    /// Fill buffers the guest offered with frames from the network.
    fn fill_receive(&mut self, state: QueueState, memory: &GuestMemory) {
        loop {
            let (available_index, entries) =
                available_entries(memory, state, self.rx_last_available);
            let Some(head) = entries.first().copied() else {
                return;
            };

            let Some(frame) = self.backend.receive() else {
                return;
            };

            // The buffer must be able to hold the frame, or the frame is dropped: writing a
            // frame larger than the buffer would write past what the guest offered.
            let Some(buffer) = descriptor(memory, state, head) else {
                return;
            };

            if frame.len() > buffer.length as usize {
                continue;
            }

            if memory
                .map()
                .write_slice(&frame, GuestAddress(buffer.address))
                .is_err()
            {
                return;
            }

            self.rx_last_available = available_index;
            self.rx_used = push_used(memory, state, self.rx_used, head, frame.len() as u32);
        }
    }
}

impl<B: NetworkBackend> VirtioBackend for VirtioNet<B> {
    fn device_id(&self) -> u32 {
        DEVICE_ID
    }

    fn device_features(&self) -> u64 {
        features::MAC | features::STATUS
    }

    fn read_config(&self, offset: u64, data: &mut [u8]) {
        // Configuration space starts with the MAC address, then the link status.
        let mut space = Vec::with_capacity(8);
        space.extend_from_slice(&self.backend.mac());
        space.extend_from_slice(&(if self.backend.link_up() { 1u16 } else { 0 }).to_le_bytes());

        for (index, byte) in data.iter_mut().enumerate() {
            *byte = space.get(offset as usize + index).copied().unwrap_or(0);
        }
    }

    fn write_config(&mut self, _offset: u64, _data: &[u8]) {
        // Every field here is the device telling the driver something, so a write changes
        // nothing. Accepting one silently is better than pretending it did something.
    }

    fn notify(&mut self, queue: u16, state: QueueState, memory: &GuestMemory) -> bool {
        match queue {
            TX_QUEUE => self.drain_transmit(state, memory),
            RX_QUEUE => self.fill_receive(state, memory),
            _ => return false,
        }

        true
    }
}

/// Read one descriptor.
fn descriptor(memory: &GuestMemory, state: QueueState, index: u16) -> Option<Descriptor> {
    crate::devices::read_descriptor(memory, state.descriptor_table, index).ok()
}

/// Walk a descriptor chain, concatenating the buffers it names.
fn read_chain(memory: &GuestMemory, state: QueueState, head: u16) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut index = head;

    // A chain longer than the table is a chain that points in a circle, and a guest that builds
    // one must not be able to hang the device.
    for _ in 0..state.num {
        let entry = descriptor(memory, state, index)?;
        let mut buffer = vec![0u8; entry.length as usize];

        if !buffer.is_empty() {
            memory
                .map()
                .read_slice(&mut buffer, GuestAddress(entry.address))
                .ok()?;
            bytes.extend_from_slice(&buffer);
        }

        if entry.flags & descriptor_flags::NEXT == 0 {
            return Some(bytes);
        }

        index = entry.next;
    }

    None
}

/// Entries the driver added to the available ring since the device last looked.
///
/// Returns the ring's new index and the entries it contained. The index moves even when no entry
/// could be read, because a driver that reused a slot expects the device to have consumed it.
fn available_entries(memory: &GuestMemory, state: QueueState, last: u16) -> (u16, Vec<u16>) {
    let Ok(index) = read_u16(memory, state.available + 2) else {
        return (last, Vec::new());
    };

    let mut entries = Vec::new();
    let mut cursor = last;

    while cursor != index {
        let slot = cursor % state.num.max(1);
        let Ok(head) = read_u16(memory, state.available + 4 + u64::from(slot) * 2) else {
            break;
        };

        entries.push(head);
        cursor = cursor.wrapping_add(1);
    }

    (index, entries)
}

/// Add an entry to the used ring and return the new index.
fn push_used(memory: &GuestMemory, state: QueueState, last: u16, head: u16, length: u32) -> u16 {
    let slot = last % state.num.max(1);
    let entry = state.used + 4 + u64::from(slot) * 8;

    if write_u32(memory, entry, u32::from(head)).is_err()
        || write_u32(memory, entry + 4, length).is_err()
        || write_u16(memory, state.used + 2, last.wrapping_add(1)).is_err()
    {
        return last;
    }

    last.wrapping_add(1)
}

fn read_u16(memory: &GuestMemory, address: u64) -> Result<u16, ()> {
    let mut bytes = [0u8; 2];
    memory
        .map()
        .read_slice(&mut bytes, GuestAddress(address))
        .map_err(|_| ())?;
    Ok(u16::from_le_bytes(bytes))
}

fn write_u16(memory: &GuestMemory, address: u64, value: u16) -> Result<(), ()> {
    memory
        .map()
        .write_slice(&value.to_le_bytes(), GuestAddress(address))
        .map_err(|_| ())
}

fn write_u32(memory: &GuestMemory, address: u64, value: u32) -> Result<(), ()> {
    memory
        .map()
        .write_slice(&value.to_le_bytes(), GuestAddress(address))
        .map_err(|_| ())
}

#[allow(dead_code)]
const _: u16 = DESCRIPTOR_SIZE;
