//! The packet layer, checked against the property that makes it usable: a checksum that verifies.

use dome_net::packets::{
    ETHERTYPE_IPV4, EthernetFrame, IcmpEcho, Ipv4Address, Ipv4Packet, MacAddress, PROTOCOL_TCP,
    PacketError, TcpSegment, UdpDatagram, checksum, tcp_flags, transport_checksum,
};

const SOURCE: Ipv4Address = Ipv4Address([10, 0, 2, 15]);
const DESTINATION: Ipv4Address = Ipv4Address([93, 184, 216, 34]);

#[test]
fn a_frame_round_trips() {
    let frame = EthernetFrame {
        destination: MacAddress::BROADCAST,
        source: MacAddress([0x52, 0x54, 0x00, 0x12, 0x34, 0x56]),
        ethertype: ETHERTYPE_IPV4,
        payload: vec![0x45, 0, 0, 20],
    };

    let bytes = frame.to_bytes();
    assert_eq!(bytes.len(), EthernetFrame::HEADER_LENGTH + 4);
    assert_eq!(EthernetFrame::parse(&bytes).expect("parse"), frame);

    // The address a broadcast frame goes to is how ARP is answered without knowing the peer.
    assert!(frame.destination.is_multicast());
    assert!(!frame.source.is_multicast());
}

#[test]
fn a_truncated_frame_is_an_error_not_a_panic() {
    let error = EthernetFrame::parse(&[0u8; 6]).unwrap_err();
    assert!(matches!(error, PacketError { .. }));
}

#[test]
fn the_ipv4_header_checksum_verifies() {
    let packet = Ipv4Packet {
        source: SOURCE,
        destination: DESTINATION,
        protocol: PROTOCOL_TCP,
        ttl: 64,
        identification: 0x1234,
        dont_fragment: true,
        payload: vec![0u8; 20],
    };

    let bytes = packet.to_bytes();
    // The defining property: summing the header, checksum included, yields zero.
    assert_eq!(checksum(&bytes[..20]), 0);

    let parsed = Ipv4Packet::parse(&bytes).expect("parse");
    assert_eq!(parsed.source, SOURCE);
    assert_eq!(parsed.destination, DESTINATION);
    assert!(parsed.dont_fragment);
    assert_eq!(parsed.ttl, 64);
}

#[test]
fn an_ipv4_packet_with_an_impossible_length_is_refused() {
    let mut bytes = vec![
        0x45, 0, 0, 10, 0, 0, 0, 0, 64, 6, 0, 0, 10, 0, 2, 15, 93, 184, 216, 34,
    ];
    bytes[3] = 10; // total length shorter than the header
    assert!(Ipv4Packet::parse(&bytes).is_err());

    bytes[0] = 0x65; // version 6
    assert!(Ipv4Packet::parse(&bytes).is_err());
}

#[test]
fn a_tcp_segment_checksum_verifies() {
    let segment = TcpSegment {
        source_port: 40000,
        destination_port: 443,
        sequence: 1,
        acknowledgement: 0,
        flags: tcp_flags::SYN,
        window: 64240,
        payload: Vec::new(),
    };

    let bytes = segment.to_bytes(SOURCE, DESTINATION);
    assert_eq!(bytes.len(), 20);

    // Embedded checksum must make the pseudo-header sum vanish.
    assert_eq!(
        transport_checksum(SOURCE, DESTINATION, PROTOCOL_TCP, &bytes),
        0
    );

    let parsed = TcpSegment::parse(&bytes).expect("parse");
    assert_eq!(parsed.destination_port, 443);
    assert_eq!(parsed.flags & tcp_flags::SYN, tcp_flags::SYN);
    assert!(parsed.payload.is_empty());
}

#[test]
fn a_udp_datagram_never_sends_a_zero_checksum() {
    let datagram = UdpDatagram {
        source_port: 5353,
        destination_port: 53,
        payload: b"dns".to_vec(),
    };

    let bytes = datagram.to_bytes(SOURCE, DESTINATION);
    let parsed = UdpDatagram::parse(&bytes).expect("parse");
    assert_eq!(parsed.payload, b"dns");

    // Zero means "not computed" in UDP, so a genuine zero has to be written as all ones.
    let written = u16::from_be_bytes([bytes[6], bytes[7]]);
    assert_ne!(written, 0);
}

#[test]
fn an_echo_request_comes_back_as_a_reply() {
    let request = IcmpEcho {
        reply: false,
        identifier: 7,
        sequence: 1,
        payload: b"ping".to_vec(),
    };

    let bytes = request.to_bytes();
    assert_eq!(bytes[0], 8);

    let parsed = IcmpEcho::parse(&bytes).expect("parse");
    assert!(!parsed.reply);
    assert_eq!(parsed.payload, b"ping");

    let reply = IcmpEcho {
        reply: true,
        payload: parsed.payload.clone(),
        ..parsed
    };
    let reply_bytes = reply.to_bytes();
    assert_eq!(reply_bytes[0], 0);
    assert_eq!(checksum(&reply_bytes), 0);
}

#[test]
fn a_mac_address_prints_the_way_people_write_it() {
    let mac = MacAddress([0x52, 0x54, 0x00, 0xab, 0xcd, 0xef]);
    assert_eq!(mac.to_string(), "52:54:00:ab:cd:ef");
}

#[test]
fn an_ipv4_address_prints_the_way_people_write_it() {
    assert_eq!(SOURCE.to_string(), "10.0.2.15");
}
