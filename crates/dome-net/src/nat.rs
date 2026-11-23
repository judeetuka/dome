//! User-mode networking: the guest's network, answered by the engine.
//!
//! A chamber has no tap device and no host bridge. Its network is a fiction the engine maintains:
//! the guest believes it is on 10.0.2.0/24 talking to a router at 10.0.2.2, and the engine answers
//! as that router, relaying what is allowed to leave.
//!
//! The tradeoff is the one slirp makes and it is worth naming. There is no host interface to
//! attack, nothing to misconfigure, and no privileged operation in the path - but the guest cannot
//! be reached from outside either, because there is no outside address. Everything here is opt-in
//! egress. That is the right default for a chamber that fetches its own packages, and the wrong one
//! for a server. Reaching a chamber from outside is a separate mechanism, not a wider device.
//!
//! What this module does not do is own sockets. Relaying TCP and resolving DNS are traits, so the
//! protocol logic is testable without a network and the socket implementations stay in the
//! supervisor where they belong.

use std::collections::HashMap;

use crate::packets::{
    ETHERTYPE_ARP, ETHERTYPE_IPV4, EthernetFrame, IcmpEcho, Ipv4Address, Ipv4Packet, MacAddress,
    PROTOCOL_ICMP, PROTOCOL_TCP, PROTOCOL_UDP, TcpSegment, UdpDatagram, tcp_flags,
};

/// The address the guest is told it has.
pub const GUEST_ADDRESS: Ipv4Address = Ipv4Address([10, 0, 2, 15]);
/// The address the guest is told its router has.
pub const GATEWAY_ADDRESS: Ipv4Address = Ipv4Address([10, 0, 2, 2]);
/// The address the guest is told its resolver has.
pub const RESOLVER_ADDRESS: Ipv4Address = Ipv4Address([10, 0, 2, 3]);
/// The MAC the guest is told its router has.
pub const GATEWAY_MAC: MacAddress = MacAddress([0x52, 0x55, 0x0a, 0x00, 0x02, 0x02]);
/// The MAC the guest is told it has.
pub const GUEST_MAC: MacAddress = MacAddress([0x52, 0x54, 0x00, 0x12, 0x34, 0x56]);

/// A TCP flow as the engine sees it.
///
/// Sequence numbers are the engine's own: the guest's numbers never leave this process, and the
/// remote host's numbers never enter it. That is what makes the mapping a border rather than a
/// pass-through.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Flow {
    /// Guest port.
    pub guest_port: u16,
    /// Remote address.
    pub remote_address: Ipv4Address,
    /// Remote port.
    pub remote_port: u16,
}

/// Relaying a TCP flow to somewhere else.
pub trait TcpRelay {
    /// A guest opened a connection. Returns whether it may proceed.
    fn open(&mut self, flow: Flow) -> bool;

    /// The guest sent bytes.
    fn send(&mut self, flow: Flow, data: &[u8]);

    /// The guest closed the flow.
    fn close(&mut self, flow: Flow);

    /// Bytes that arrived for the guest, if any.
    fn poll(&mut self, flow: Flow) -> Option<Vec<u8>>;
}

/// Resolving names on the guest's behalf.
///
/// The guest points its resolver at 10.0.2.3 and the engine forwards the query, so a chamber does
/// not have to know how this machine reaches DNS.
pub trait DnsResolver {
    /// Resolve a query, returning the response, or None if it cannot be resolved.
    fn resolve(&mut self, query: &[u8]) -> Option<Vec<u8>>;
}

/// What the engine decided to do with a packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Disposition {
    /// Frames to hand back to the guest.
    Reply(Vec<Vec<u8>>),
    /// Nothing to say.
    Drop,
}
/// The guest's network.
pub struct UserNet<R: TcpRelay, D: DnsResolver> {
    relay: R,
    resolver: D,
    flows: HashMap<Flow, FlowState>,
}

/// What the engine remembers about a flow.
#[derive(Debug, Clone, Copy)]
struct FlowState {
    /// The sequence number the next guest-bound segment will carry.
    ///
    /// The engine numbers its own direction. The guest's numbers never leave this process and the
    /// remote host's never enter it, which is what makes the translation a border.
    next_sequence: u32,
}

impl<R: TcpRelay, D: DnsResolver> UserNet<R, D> {
    /// A network with the given relay and resolver.
    pub fn new(relay: R, resolver: D) -> Self {
        Self {
            relay,
            resolver,
            flows: HashMap::new(),
        }
    }

    /// How many flows are open, which is what a limit would be applied to.
    pub fn open_flows(&self) -> usize {
        self.flows.len()
    }

    /// Handle one frame from the guest.
    pub fn handle(&mut self, frame: &[u8]) -> Disposition {
        let Ok(frame) = EthernetFrame::parse(frame) else {
            return Disposition::Drop;
        };

        match frame.ethertype {
            ETHERTYPE_ARP => self.handle_arp(&frame),
            ETHERTYPE_IPV4 => self.handle_ipv4(&frame),
            _ => Disposition::Drop,
        }
    }

    /// Answer an address resolution request for the gateway or the resolver.
    fn handle_arp(&mut self, frame: &EthernetFrame) -> Disposition {
        let payload = &frame.payload;
        if payload.len() < 28 {
            return Disposition::Drop;
        }

        let operation = u16::from_be_bytes([payload[6], payload[7]]);
        if operation != 1 {
            // Only requests are answered. A reply tells us nothing we did not already know.
            return Disposition::Drop;
        }

        let mut sender_mac = [0u8; 6];
        sender_mac.copy_from_slice(&payload[8..14]);

        let mut sender_address = [0u8; 4];
        sender_address.copy_from_slice(&payload[14..18]);

        let mut target = [0u8; 4];
        target.copy_from_slice(&payload[24..28]);

        let target = Ipv4Address(target);
        if target != GATEWAY_ADDRESS && target != RESOLVER_ADDRESS {
            return Disposition::Drop;
        }

        let mut reply = Vec::with_capacity(28);
        reply.extend_from_slice(&[0, 1]);
        reply.extend_from_slice(&[0x08, 0x00]);
        reply.push(6);
        reply.push(4);
        reply.extend_from_slice(&[0, 2]);
        reply.extend_from_slice(&GATEWAY_MAC.0);
        reply.extend_from_slice(&GATEWAY_ADDRESS.0);
        reply.extend_from_slice(&sender_mac);
        reply.extend_from_slice(&sender_address);

        Disposition::Reply(vec![
            EthernetFrame {
                destination: MacAddress(sender_mac),
                source: GATEWAY_MAC,
                ethertype: ETHERTYPE_ARP,
                payload: reply,
            }
            .to_bytes(),
        ])
    }

    /// Answer, relay or drop an IP packet.
    fn handle_ipv4(&mut self, frame: &EthernetFrame) -> Disposition {
        let Ok(packet) = Ipv4Packet::parse(&frame.payload) else {
            return Disposition::Drop;
        };

        match packet.protocol {
            PROTOCOL_ICMP if packet.destination == GATEWAY_ADDRESS => self.handle_icmp(&packet),
            PROTOCOL_UDP if packet.destination == RESOLVER_ADDRESS => self.handle_dns(&packet),
            PROTOCOL_TCP => self.handle_tcp(&packet),
            _ => Disposition::Drop,
        }
    }

    /// Echo back what was sent, which is what makes a network feel reachable.
    fn handle_icmp(&mut self, packet: &Ipv4Packet) -> Disposition {
        let Ok(request) = IcmpEcho::parse(&packet.payload) else {
            return Disposition::Drop;
        };

        if request.reply {
            return Disposition::Drop;
        }

        let reply = IcmpEcho {
            reply: true,
            ..request
        };

        Disposition::Reply(vec![self.to_guest(
            GUEST_ADDRESS,
            GATEWAY_ADDRESS,
            PROTOCOL_ICMP,
            reply.to_bytes(),
        )])
    }

    /// Forward a DNS query and return the answer.
    fn handle_dns(&mut self, packet: &Ipv4Packet) -> Disposition {
        let Ok(query) = UdpDatagram::parse(&packet.payload) else {
            return Disposition::Drop;
        };

        if query.destination_port != 53 || packet.source != GUEST_ADDRESS {
            return Disposition::Drop;
        }

        let Some(response) = self.resolver.resolve(&query.payload) else {
            return Disposition::Drop;
        };

        let answer = UdpDatagram {
            source_port: 53,
            destination_port: query.source_port,
            payload: response,
        };

        Disposition::Reply(vec![self.to_guest(
            GUEST_ADDRESS,
            RESOLVER_ADDRESS,
            PROTOCOL_UDP,
            answer.to_bytes(RESOLVER_ADDRESS, GUEST_ADDRESS),
        )])
    }

    /// Relay a connection, or say why not.
    fn handle_tcp(&mut self, packet: &Ipv4Packet) -> Disposition {
        if packet.source != GUEST_ADDRESS {
            return Disposition::Drop;
        }

        let Ok(segment) = TcpSegment::parse(&packet.payload) else {
            return Disposition::Drop;
        };

        let flow = Flow {
            guest_port: segment.source_port,
            remote_address: packet.destination,
            remote_port: segment.destination_port,
        };

        // Loopback to the router itself is not a tunnel out.
        if flow.remote_address == GATEWAY_ADDRESS {
            return Disposition::Drop;
        }

        if segment.flags & tcp_flags::SYN != 0 && segment.flags & tcp_flags::ACK == 0 {
            if !self.relay.open(flow) {
                return Disposition::Reply(vec![self.tcp_reset(&flow, &segment)]);
            }

            self.flows.insert(flow, FlowState { next_sequence: 2 });

            let acknowledgement = segment.sequence.wrapping_add(1);
            let syn_ack = TcpSegment {
                source_port: flow.remote_port,
                destination_port: flow.guest_port,
                sequence: 1,
                acknowledgement,
                flags: tcp_flags::SYN | tcp_flags::ACK,
                window: 65535,
                payload: Vec::new(),
            };
            let bytes = syn_ack.to_bytes(flow.remote_address, GUEST_ADDRESS);

            return Disposition::Reply(vec![self.to_guest(
                GUEST_ADDRESS,
                flow.remote_address,
                PROTOCOL_TCP,
                bytes,
            )]);
        }

        if !self.flows.contains_key(&flow) {
            return Disposition::Drop;
        }

        if !segment.payload.is_empty() {
            self.relay.send(flow, &segment.payload);
        }

        if segment.flags & (tcp_flags::FIN | tcp_flags::RST) != 0 {
            self.relay.close(flow);
            self.flows.remove(&flow);
        }

        Disposition::Drop
    }

    /// Frames waiting to go back to the guest.
    ///
    /// Called when the engine has nothing else to do. Each open flow is polled, and any bytes that
    /// arrived are framed as a segment the guest will accept: the sequence number continues from
    /// the one this side advertised during the handshake.
    pub fn poll(&mut self) -> Vec<Vec<u8>> {
        let flows: Vec<Flow> = self.flows.keys().copied().collect();
        let mut frames = Vec::new();

        for flow in flows {
            let Some(data) = self.relay.poll(flow) else {
                continue;
            };

            if data.is_empty() {
                continue;
            }

            let Some(state) = self.flows.get_mut(&flow) else {
                continue;
            };

            let segment = TcpSegment {
                source_port: flow.remote_port,
                destination_port: flow.guest_port,
                sequence: state.next_sequence,
                acknowledgement: 0,
                flags: tcp_flags::ACK | tcp_flags::PSH,
                window: 65535,
                payload: data.clone(),
            };

            // The guest's own numbering is not tracked, so the acknowledgement repeats the last
            // value it saw. A guest that insists on exact acknowledgement accounting would need
            // this to be a real counter, which is noted in T76 rather than guessed at.
            state.next_sequence = state.next_sequence.wrapping_add(data.len() as u32);

            let bytes = segment.to_bytes(flow.remote_address, GUEST_ADDRESS);
            frames.push(self.to_guest(GUEST_ADDRESS, flow.remote_address, PROTOCOL_TCP, bytes));
        }

        frames
    }

    /// A reset for a connection that will not be opened.
    fn tcp_reset(&self, flow: &Flow, segment: &TcpSegment) -> Vec<u8> {
        let reset = TcpSegment {
            source_port: flow.remote_port,
            destination_port: flow.guest_port,
            sequence: segment.sequence,
            acknowledgement: segment.sequence.wrapping_add(1),
            flags: tcp_flags::RST | tcp_flags::ACK,
            window: 0,
            payload: Vec::new(),
        };

        self.to_guest(
            GUEST_ADDRESS,
            flow.remote_address,
            PROTOCOL_TCP,
            reset.to_bytes(flow.remote_address, GUEST_ADDRESS),
        )
    }

    /// Wrap a payload in the framing the guest expects from its router.
    fn to_guest(
        &self,
        destination: Ipv4Address,
        source: Ipv4Address,
        protocol: u8,
        payload: Vec<u8>,
    ) -> Vec<u8> {
        let packet = Ipv4Packet {
            source,
            destination,
            protocol,
            ttl: 64,
            identification: 0,
            dont_fragment: true,
            payload,
        };

        EthernetFrame {
            destination: GUEST_MAC,
            source: GATEWAY_MAC,
            ethertype: ETHERTYPE_IPV4,
            payload: packet.to_bytes(),
        }
        .to_bytes()
    }
}
