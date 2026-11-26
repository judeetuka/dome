//! The network device, driven the way a guest drives it.
//!
//! No hypervisor and no network: a queue is a ring in memory and a frame is bytes, so the device
//! can be proven with a buffer, a descriptor and a fake backend.

use dome_core::MemorySize;
use dome_engine::devices::virtio_mmio::{QueueState, VirtioBackend, reg};
use dome_engine::devices::virtio_net::{
    DEVICE_ID, NetworkBackend, RX_QUEUE, TX_QUEUE, VirtioNet, features,
};
use dome_engine::memory::GuestMemory;
use vm_memory::{Bytes, GuestAddress};

const DESCRIPTOR_TABLE: u64 = 0x1_0000;
const AVAILABLE_RING: u64 = 0x2_0000;
const USED_RING: u64 = 0x3_0000;
const BUFFER: u64 = 0x4_0000;
const QUEUE_SIZE: u16 = 8;

struct FakeNetwork {
    mac: [u8; 6],
    transmitted: Vec<Vec<u8>>,
    to_receive: Vec<Vec<u8>>,
}

impl FakeNetwork {
    fn new() -> Self {
        Self {
            mac: [0x52, 0x54, 0x00, 0xaa, 0xbb, 0xcc],
            transmitted: Vec::new(),
            to_receive: Vec::new(),
        }
    }
}

impl NetworkBackend for FakeNetwork {
    fn mac(&self) -> [u8; 6] {
        self.mac
    }

    fn transmit(&mut self, frame: &[u8]) {
        self.transmitted.push(frame.to_vec());
    }

    fn receive(&mut self) -> Option<Vec<u8>> {
        if self.to_receive.is_empty() {
            None
        } else {
            Some(self.to_receive.remove(0))
        }
    }
}

fn memory() -> GuestMemory {
    GuestMemory::allocate(MemorySize::from_mib(16).expect("16 MiB")).expect("memory")
}

fn queue_state() -> QueueState {
    QueueState {
        num: QUEUE_SIZE,
        ready: true,
        descriptor_table: DESCRIPTOR_TABLE,
        available: AVAILABLE_RING,
        used: USED_RING,
    }
}

/// Write one descriptor.
fn write_descriptor(memory: &GuestMemory, index: u16, address: u64, length: u32) {
    let mut bytes = Vec::with_capacity(16);
    bytes.extend_from_slice(&address.to_le_bytes());
    bytes.extend_from_slice(&length.to_le_bytes());
    bytes.extend_from_slice(&0u16.to_le_bytes());
    bytes.extend_from_slice(&0u16.to_le_bytes());

    let at = DESCRIPTOR_TABLE + u64::from(index) * 16;
    memory
        .map()
        .write_slice(&bytes, GuestAddress(at))
        .expect("descriptor");
}

/// Offer one buffer to the device through the available ring.
fn offer(memory: &GuestMemory, ring_index: u16, head: u16) {
    memory
        .map()
        .write_slice(
            &head.to_le_bytes(),
            GuestAddress(AVAILABLE_RING + 4 + u64::from(ring_index) * 2),
        )
        .expect("ring entry");
    memory
        .map()
        .write_slice(
            &(ring_index + 1).to_le_bytes(),
            GuestAddress(AVAILABLE_RING + 2),
        )
        .expect("ring index");
}

fn used_index(memory: &GuestMemory) -> u16 {
    let mut bytes = [0u8; 2];
    memory
        .map()
        .read_slice(&mut bytes, GuestAddress(USED_RING + 2))
        .expect("used");
    u16::from_le_bytes(bytes)
}

#[test]
fn a_guest_frame_reaches_the_network() {
    let memory = memory();
    let mut device = VirtioNet::new(FakeNetwork::new());

    // The guest writes a virtio header and then a frame into one buffer.
    let payload = b"a frame from the guest".to_vec();
    let mut frame = vec![0u8; 12];
    frame.extend_from_slice(&payload);
    memory
        .map()
        .write_slice(&frame, GuestAddress(BUFFER))
        .expect("frame");
    write_descriptor(&memory, 0, BUFFER, frame.len() as u32);
    offer(&memory, 0, 0);

    assert!(device.notify(TX_QUEUE, queue_state(), &memory));

    assert_eq!(device.backend().transmitted, vec![payload]);
    assert_eq!(
        used_index(&memory),
        1,
        "the guest is told its buffer came back"
    );
}

#[test]
fn a_frame_from_the_network_lands_in_the_guest_buffer() {
    let memory = memory();
    let mut backend = FakeNetwork::new();
    backend.to_receive.push(b"inbound".to_vec());

    let mut device = VirtioNet::new(backend);

    write_descriptor(&memory, 0, BUFFER, 2048);
    offer(&memory, 0, 0);

    device.notify(RX_QUEUE, queue_state(), &memory);

    // The frame arrives behind the virtio header the driver expects in front of every packet.
    let mut buffer = vec![0u8; 19];
    memory
        .map()
        .read_slice(&mut buffer, GuestAddress(BUFFER))
        .expect("buffer");
    assert_eq!(
        &buffer[..10],
        &[0u8; 10],
        "no offload is offered, so flags and lengths are zero"
    );
    assert_eq!(
        &buffer[10..12],
        &1u16.to_le_bytes(),
        "one buffer holds the frame"
    );
    assert_eq!(&buffer[12..], b"inbound");
    assert_eq!(used_index(&memory), 1);
}

#[test]
fn a_frame_larger_than_the_offered_buffer_is_not_written() {
    let memory = memory();
    let mut backend = FakeNetwork::new();
    backend.to_receive.push(vec![0xAB; 64]);

    let mut device = VirtioNet::new(backend);

    // The guest offered four bytes for a sixty-four byte frame.
    write_descriptor(&memory, 0, BUFFER, 4);
    offer(&memory, 0, 0);

    device.notify(RX_QUEUE, queue_state(), &memory);

    // Nothing may be written past what was offered: the frame is dropped instead.
    let mut probe = [0u8; 4];
    memory
        .map()
        .read_slice(&mut probe, GuestAddress(BUFFER))
        .expect("probe");
    assert_eq!(probe, [0, 0, 0, 0], "the buffer is untouched");
}

#[test]
fn the_device_announces_its_mac_and_link_state() {
    let device = VirtioNet::new(FakeNetwork::new());
    assert_eq!(device.device_id(), DEVICE_ID);
    assert_eq!(device.device_features() & features::MAC, features::MAC);

    let mut config = [0u8; 8];
    device.read_config(0, &mut config);
    assert_eq!(&config[0..6], &[0x52, 0x54, 0x00, 0xaa, 0xbb, 0xcc]);
    assert_eq!(u16::from_le_bytes([config[6], config[7]]), 1, "link is up");
}

#[test]
fn an_unknown_queue_is_not_claimed() {
    let memory = memory();
    let mut device = VirtioNet::new(FakeNetwork::new());
    assert!(!device.notify(7, queue_state(), &memory));
}

#[test]
fn a_chain_longer_than_the_table_is_not_followed() {
    // Every descriptor points at the next one, so the chain never ends. The device must stop
    // rather than loop: the descriptor table is guest memory, and a guest can lie in it.
    let memory = memory();
    let device = VirtioNet::new(FakeNetwork::new());

    for index in 0..QUEUE_SIZE {
        let next = (index + 1) % QUEUE_SIZE;
        let mut bytes = Vec::with_capacity(16);
        bytes.extend_from_slice(&BUFFER.to_le_bytes());
        bytes.extend_from_slice(&4u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes()); // NEXT
        bytes.extend_from_slice(&next.to_le_bytes());
        let at = DESCRIPTOR_TABLE + u64::from(index) * 16;
        memory
            .map()
            .write_slice(&bytes, GuestAddress(at))
            .expect("descriptor");
    }

    offer(&memory, 0, 0);
    let mut device = device;
    device.notify(TX_QUEUE, queue_state(), &memory);

    // Nothing was transmitted from a chain that could never terminate.
    assert!(device.backend().transmitted.is_empty());
}

#[test]
fn a_ready_queue_is_required_before_the_device_touches_memory() {
    // The transport enforces this, and the device agrees: an unready queue has no ring to read.
    let memory = memory();
    let device = VirtioNet::new(FakeNetwork::new());
    let mut state = queue_state();
    state.ready = false;

    // Calling notify directly bypasses the transport, so this documents the contract rather than
    // testing the guard: with no ring written, there is nothing to find and nothing is sent.
    let mut device = device;
    device.notify(TX_QUEUE, state, &memory);
    assert!(device.backend().transmitted.is_empty());
}

#[test]
fn the_transport_routes_a_doorbell_to_the_network_device() {
    use dome_engine::devices::virtio_mmio::VirtioMmio;
    use dome_engine::devices::{ChamberIo, Device};

    let memory = memory();
    let mut transport = VirtioMmio::new(0xd000_0000, VirtioNet::new(FakeNetwork::new()));

    // Configure queue 1 the way a driver would.
    transport.write_u32(reg::QUEUE_SEL, 1);
    transport.write_u32(reg::QUEUE_NUM, u32::from(QUEUE_SIZE));
    transport.write_u32(reg::QUEUE_DESC_LOW, DESCRIPTOR_TABLE as u32);
    transport.write_u32(reg::QUEUE_DRIVER_LOW, AVAILABLE_RING as u32);
    transport.write_u32(reg::QUEUE_DEVICE_LOW, USED_RING as u32);
    transport.write_u32(reg::QUEUE_READY, 1);

    let payload = b"through the transport".to_vec();
    let mut frame = vec![0u8; 12];
    frame.extend_from_slice(&payload);
    memory
        .map()
        .write_slice(&frame, GuestAddress(BUFFER))
        .expect("frame");
    write_descriptor(&memory, 0, BUFFER, frame.len() as u32);
    offer(&memory, 0, 0);

    transport.write_u32(reg::QUEUE_NOTIFY, 1);
    let queue = transport.take_doorbell().expect("pending");
    transport.service_doorbell(queue, &memory);

    assert_eq!(transport.backend().backend().transmitted, vec![payload]);
    let _ = ChamberIo::with_console();
}
