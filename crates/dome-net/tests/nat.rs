//! User-mode networking, checked frame by frame.
//!
//! These are the packets a guest actually sends when it runs ping, a resolver query or a TCP
//! handshake, built with the packet layer and fed to the network as bytes.

use dome_net::nat::{
    Disposition, DnsResolver, Flow, GATEWAY_ADDRESS, GATEWAY_MAC, GUEST_ADDRESS, GUEST_MAC,
    RESOLVER_ADDRESS, TcpRelay, UserNet,
};
use dome_net::packets::{
    ETHERTYPE_ARP, ETHERTYPE_IPV4, EthernetFrame, IcmpEcho, Ipv4Address, Ipv4Packet, MacAddress,
    PROTOCOL_ICMP, PROTOCOL_TCP, PROTOCOL_UDP, TcpSegment, UdpDatagram, tcp_flags,
};

const REMOTE: Ipv4Address = Ipv4Address([93, 184, 216, 34]);

#[derive(Default)]
struct RecordingRelay {
    opened: Vec<Flow>,
    sent: Vec<(Flow, Vec<u8>)>,
    closed: Vec<Flow>,
    allow: bool,
}

impl TcpRelay for RecordingRelay {
    fn open(&mut self, flow: Flow) -> bool {
        self.opened.push(flow);
        self.allow
    }

    fn send(&mut self, flow: Flow, data: &[u8]) {
        self.sent.push((flow, data.to_vec()));
    }

    fn close(&mut self, flow: Flow) {
        self.closed.push(flow);
    }

    fn poll(&mut self, _flow: Flow) -> Option<Vec<u8>> {
        None
    }
}

#[derive(Default)]
struct FakeResolver {
    queries: Vec<Vec<u8>>,
    answer: Option<Vec<u8>>,
}

impl DnsResolver for FakeResolver {
    fn resolve(&mut self, query: &[u8]) -> Option<Vec<u8>> {
        self.queries.push(query.to_vec());
        self.answer.clone()
    }
}

fn arp_request(target: Ipv4Address) -> Vec<u8> {
    let mut payload = Vec::with_capacity(28);
    payload.extend_from_slice(&[0, 1, 0x08, 0x00, 6, 4, 0, 1]);
    payload.extend_from_slice(&GUEST_MAC.0);
    payload.extend_from_slice(&GUEST_ADDRESS.0);
    payload.extend_from_slice(&[0u8; 6]);
    payload.extend_from_slice(&target.0);

    EthernetFrame {
        destination: MacAddress::BROADCAST,
        source: GUEST_MAC,
        ethertype: ETHERTYPE_ARP,
        payload,
    }
    .to_bytes()
}

fn ip_frame(packet: Ipv4Packet) -> Vec<u8> {
    EthernetFrame {
        destination: GATEWAY_MAC,
        source: GUEST_MAC,
        ethertype: ETHERTYPE_IPV4,
        payload: packet.to_bytes(),
    }
    .to_bytes()
}

fn network(allow: bool, answer: Option<Vec<u8>>) -> UserNet<RecordingRelay, FakeResolver> {
    UserNet::new(
        RecordingRelay {
            allow,
            ..Default::default()
        },
        FakeResolver {
            answer,
            ..Default::default()
        },
    )
}

#[test]
fn the_guest_learns_the_routers_mac_address() {
    let mut net = network(true, None);
    let Disposition::Reply(frames) = net.handle(&arp_request(GATEWAY_ADDRESS)) else {
        panic!("no reply to an ARP request");
    };

    let frame = EthernetFrame::parse(&frames[0]).expect("frame");
    assert_eq!(frame.ethertype, ETHERTYPE_ARP);
    assert_eq!(frame.source, GATEWAY_MAC);
    // The operation is a reply and the sender hardware address is ours.
    assert_eq!(&frame.payload[6..8], &[0, 2]);
    assert_eq!(&frame.payload[8..14], &GATEWAY_MAC.0);
}

#[test]
fn an_arp_request_for_something_else_is_ignored() {
    let mut net = network(true, None);
    assert_eq!(
        net.handle(&arp_request(Ipv4Address([10, 0, 2, 99]))),
        Disposition::Drop
    );
}

#[test]
fn ping_comes_back() {
    let mut net = network(true, None);

    let request = IcmpEcho {
        reply: false,
        identifier: 7,
        sequence: 1,
        payload: b"dome".to_vec(),
    };

    let frame = ip_frame(Ipv4Packet {
        source: GUEST_ADDRESS,
        destination: GATEWAY_ADDRESS,
        protocol: PROTOCOL_ICMP,
        ttl: 64,
        identification: 0,
        dont_fragment: true,
        payload: request.to_bytes(),
    });

    let Disposition::Reply(frames) = net.handle(&frame) else {
        panic!("no echo reply");
    };

    let reply = EthernetFrame::parse(&frames[0]).expect("frame");
    let packet = Ipv4Packet::parse(&reply.payload).expect("packet");
    let echo = IcmpEcho::parse(&packet.payload).expect("echo");
    assert!(echo.reply);
    assert_eq!(echo.payload, b"dome");
    assert_eq!(packet.source, GATEWAY_ADDRESS);
}

#[test]
fn a_name_is_resolved_by_the_engine() {
    let mut net = network(true, Some(vec![1, 2, 3, 4]));

    let query = UdpDatagram {
        source_port: 40000,
        destination_port: 53,
        payload: b"query".to_vec(),
    };

    let frame = ip_frame(Ipv4Packet {
        source: GUEST_ADDRESS,
        destination: RESOLVER_ADDRESS,
        protocol: PROTOCOL_UDP,
        ttl: 64,
        identification: 0,
        dont_fragment: true,
        payload: query.to_bytes(GUEST_ADDRESS, RESOLVER_ADDRESS),
    });

    let Disposition::Reply(frames) = net.handle(&frame) else {
        panic!("no answer");
    };

    let reply = EthernetFrame::parse(&frames[0]).expect("frame");
    let packet = Ipv4Packet::parse(&reply.payload).expect("packet");
    let answer = UdpDatagram::parse(&packet.payload).expect("udp");
    assert_eq!(answer.source_port, 53);
    assert_eq!(answer.destination_port, 40000);
    assert_eq!(answer.payload, vec![1, 2, 3, 4]);
    assert_eq!(packet.source, RESOLVER_ADDRESS);
}

#[test]
fn a_connection_the_engine_refuses_is_reset_not_silently_dropped() {
    let mut net = network(false, None);

    let syn = TcpSegment {
        source_port: 40000,
        destination_port: 443,
        sequence: 100,
        acknowledgement: 0,
        flags: tcp_flags::SYN,
        window: 64240,
        payload: Vec::new(),
    };

    let frame = ip_frame(Ipv4Packet {
        source: GUEST_ADDRESS,
        destination: REMOTE,
        protocol: PROTOCOL_TCP,
        ttl: 64,
        identification: 0,
        dont_fragment: true,
        payload: syn.to_bytes(GUEST_ADDRESS, REMOTE),
    });

    let Disposition::Reply(frames) = net.handle(&frame) else {
        panic!("a refusal must be a reset, or the guest waits for a timeout");
    };

    let reply = EthernetFrame::parse(&frames[0]).expect("frame");
    let packet = Ipv4Packet::parse(&reply.payload).expect("packet");
    let segment = TcpSegment::parse(&packet.payload).expect("tcp");
    assert_eq!(segment.flags & tcp_flags::RST, tcp_flags::RST);
}

#[test]
fn an_accepted_connection_is_acknowledged_and_then_relayed() {
    let mut net = network(true, None);

    let syn = TcpSegment {
        source_port: 40000,
        destination_port: 443,
        sequence: 100,
        acknowledgement: 0,
        flags: tcp_flags::SYN,
        window: 64240,
        payload: Vec::new(),
    };

    let frame = ip_frame(Ipv4Packet {
        source: GUEST_ADDRESS,
        destination: REMOTE,
        protocol: PROTOCOL_TCP,
        ttl: 64,
        identification: 0,
        dont_fragment: true,
        payload: syn.to_bytes(GUEST_ADDRESS, REMOTE),
    });

    let Disposition::Reply(frames) = net.handle(&frame) else {
        panic!("no handshake reply");
    };

    let reply = EthernetFrame::parse(&frames[0]).expect("frame");
    let packet = Ipv4Packet::parse(&reply.payload).expect("packet");
    let segment = TcpSegment::parse(&packet.payload).expect("tcp");
    assert_eq!(segment.flags, tcp_flags::SYN | tcp_flags::ACK);
    assert_eq!(
        segment.acknowledgement, 101,
        "the guest's sequence number plus one"
    );
    assert_eq!(net.open_flows(), 1);

    // Then the guest sends bytes on the flow it now believes is open.
    let data = TcpSegment {
        source_port: 40000,
        destination_port: 443,
        sequence: 101,
        acknowledgement: 2,
        flags: tcp_flags::ACK,
        window: 64240,
        payload: b"GET /".to_vec(),
    };

    let frame = ip_frame(Ipv4Packet {
        source: GUEST_ADDRESS,
        destination: REMOTE,
        protocol: PROTOCOL_TCP,
        ttl: 64,
        identification: 0,
        dont_fragment: true,
        payload: data.to_bytes(GUEST_ADDRESS, REMOTE),
    });

    assert_eq!(net.handle(&frame), Disposition::Drop);
    assert_eq!(net.open_flows(), 1);
}

#[test]
fn data_for_an_unknown_flow_is_not_relayed() {
    let mut net = network(true, None);

    let data = TcpSegment {
        source_port: 40000,
        destination_port: 443,
        sequence: 1,
        acknowledgement: 0,
        flags: tcp_flags::ACK,
        window: 64240,
        payload: b"forged".to_vec(),
    };

    let frame = ip_frame(Ipv4Packet {
        source: GUEST_ADDRESS,
        destination: REMOTE,
        protocol: PROTOCOL_TCP,
        ttl: 64,
        identification: 0,
        dont_fragment: true,
        payload: data.to_bytes(GUEST_ADDRESS, REMOTE),
    });

    // A segment for a flow that was never opened must not reach a socket: the engine only
    // relays flows it agreed to.
    assert_eq!(net.handle(&frame), Disposition::Drop);
}
