//! The socket side of user-mode networking.
//!
//! The protocol logic in dome-net is deliberately free of sockets: it decides what a packet means.
//! This module is where meaning becomes a connection, and it lives in the supervisor because a
//! socket is a host resource and the device model is not allowed to own one.
//!
//! Everything here is non-blocking. A chamber that stops reading must not stall the thread that
//! serves it, and a connect that takes thirty seconds must not become a thirty second stall in the
//! guest's handshake.

use std::collections::HashMap;
use std::io::{ErrorKind, Read, Write};
use std::net::{IpAddr, Ipv4Addr, Shutdown, SocketAddr, TcpStream, UdpSocket};
use std::time::Duration;

use std::collections::VecDeque;

use dome_engine::devices::virtio_net::NetworkBackend;
use dome_net::nat::{DnsResolver, Flow, TcpRelay};

/// How long a DNS query may take before it is abandoned.
const DNS_TIMEOUT: Duration = Duration::from_secs(5);

/// How long a connection attempt may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// One relayed connection.
struct Connection {
    stream: TcpStream,
}

/// Relays guest connections through host sockets.
#[derive(Default)]
pub struct SocketRelay {
    connections: HashMap<Flow, Connection>,
    /// Whether connections may leave at all, which is the perimeter's decision.
    allowed: bool,
}

impl SocketRelay {
    /// A relay that permits outbound connections.
    pub fn permitting() -> Self {
        Self {
            connections: HashMap::new(),
            allowed: true,
        }
    }

    /// A relay that refuses everything, for an isolated chamber.
    pub fn isolated() -> Self {
        Self {
            connections: HashMap::new(),
            allowed: false,
        }
    }

    /// Connections currently open.
    pub fn len(&self) -> usize {
        self.connections.len()
    }

    /// Whether no connection is open.
    pub fn is_empty(&self) -> bool {
        self.connections.is_empty()
    }
}

impl TcpRelay for SocketRelay {
    fn open(&mut self, flow: Flow) -> bool {
        if !self.allowed || self.connections.contains_key(&flow) {
            return false;
        }

        let address = SocketAddr::new(
            IpAddr::V4(Ipv4Addr::from(flow.remote_address.0)),
            flow.remote_port,
        );

        // A connect that never completes is refused rather than left hanging: the guest is
        // waiting on a handshake it will otherwise give up on its own terms.
        let Ok(stream) = TcpStream::connect_timeout(&address, CONNECT_TIMEOUT) else {
            return false;
        };

        if stream.set_nonblocking(true).is_err() {
            return false;
        }

        // Small writes should leave promptly: a relayed request is not a download.
        let _ = stream.set_nodelay(true);

        self.connections.insert(flow, Connection { stream });
        true
    }

    fn send(&mut self, flow: Flow, data: &[u8]) {
        if let Some(connection) = self.connections.get_mut(&flow) {
            // A partial write is not an error to report to the guest: TCP is a stream, and the
            // rest is retried on the next segment. A broken pipe does end the flow.
            match connection.stream.write_all(data) {
                Ok(()) => {}
                Err(error) if error.kind() == ErrorKind::WouldBlock => {}
                Err(_) => {
                    self.connections.remove(&flow);
                }
            }
        }
    }

    fn close(&mut self, flow: Flow) {
        if let Some(connection) = self.connections.remove(&flow) {
            let _ = connection.stream.shutdown(Shutdown::Both);
        }
    }

    fn poll(&mut self, flow: Flow) -> Option<Vec<u8>> {
        let connection = self.connections.get_mut(&flow)?;

        let mut buffer = vec![0u8; 16 * 1024];
        match connection.stream.read(&mut buffer) {
            Ok(0) => {
                // The far end closed.
                self.connections.remove(&flow);
                None
            }
            Ok(read) => {
                buffer.truncate(read);
                Some(buffer)
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => None,
            Err(_) => {
                self.connections.remove(&flow);
                None
            }
        }
    }
}

/// Forwards guest DNS queries to a real resolver.
pub struct SystemResolver {
    upstream: SocketAddr,
}

impl SystemResolver {
    /// A resolver that asks the given server.
    pub fn new(upstream: SocketAddr) -> Self {
        Self { upstream }
    }

    /// A resolver that asks a public server, so a chamber does not depend on the host having one
    /// configured in a particular file.
    pub fn from_host() -> Self {
        Self::new(SocketAddr::new(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)), 53))
    }
}

impl DnsResolver for SystemResolver {
    fn resolve(&mut self, query: &[u8]) -> Option<Vec<u8>> {
        let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
        socket.set_read_timeout(Some(DNS_TIMEOUT)).ok()?;

        socket.send_to(query, self.upstream).ok()?;

        let mut response = vec![0u8; 4096];
        let (read, _) = socket.recv_from(&mut response).ok()?;
        response.truncate(read);

        Some(response)
    }
}

/// The user-mode network as a device backend.
///
/// The device moves frames; this decides what they mean. One direction is synchronous (a request
/// that can be answered from the frame alone: ARP, ICMP, DNS, a handshake). The other arrives
/// later, which is why receive polls the flows rather than waiting for a frame to trigger it.
pub struct UserNetBackend {
    net: dome_net::nat::UserNet<SocketRelay, SystemResolver>,
    outbox: VecDeque<Vec<u8>>,
}

impl UserNetBackend {
    /// A backend with the given perimeter.
    pub fn new(relay: SocketRelay) -> Self {
        Self {
            net: dome_net::nat::UserNet::new(relay, SystemResolver::from_host()),
            outbox: VecDeque::new(),
        }
    }

    /// A backend with a resolver of the caller's choosing, for tests.
    pub fn with_resolver(relay: SocketRelay, resolver: SystemResolver) -> Self {
        Self {
            net: dome_net::nat::UserNet::new(relay, resolver),
            outbox: VecDeque::new(),
        }
    }

    /// How many flows the guest has open.
    pub fn open_flows(&self) -> usize {
        self.net.open_flows()
    }
}

impl NetworkBackend for UserNetBackend {
    fn mac(&self) -> [u8; 6] {
        dome_net::nat::GUEST_MAC.0
    }

    fn transmit(&mut self, frame: &[u8]) {
        if let dome_net::nat::Disposition::Reply(frames) = self.net.handle(frame) {
            self.outbox.extend(frames);
        }
    }

    fn receive(&mut self) -> Option<Vec<u8>> {
        if let Some(frame) = self.outbox.pop_front() {
            return Some(frame);
        }

        // Nothing queued: ask the flows whether anything arrived while we were idle.
        self.outbox.extend(self.net.poll());
        self.outbox.pop_front()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn flow(port: u16) -> Flow {
        Flow {
            guest_port: 40000,
            remote_address: dome_net::packets::Ipv4Address([127, 0, 0, 1]),
            remote_port: port,
        }
    }

    #[test]
    fn a_connection_is_relayed_both_ways() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("listener");
        let port = listener.local_addr().expect("address").port();

        let mut relay = SocketRelay::permitting();
        assert!(relay.open(flow(port)), "the connection opens");

        let (mut server, _) = listener.accept().expect("accept");
        relay.send(flow(port), b"hello");

        let mut received = [0u8; 5];
        server.read_exact(&mut received).expect("read");
        assert_eq!(&received, b"hello");

        server.write_all(b"world").expect("write");

        // The read end is non-blocking, so retry until the bytes arrive.
        let mut answer = None;
        for _ in 0..100 {
            if let Some(bytes) = relay.poll(flow(port)) {
                answer = Some(bytes);
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }

        assert_eq!(answer.as_deref(), Some(&b"world"[..]));
    }

    #[test]
    fn an_isolated_relay_opens_nothing() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("listener");
        let port = listener.local_addr().expect("address").port();

        let mut relay = SocketRelay::isolated();
        assert!(!relay.open(flow(port)));
        assert!(relay.is_empty());
    }

    #[test]
    fn a_resolver_forwards_the_query_and_returns_the_answer() {
        // A UDP socket standing in for a nameserver.
        let upstream = UdpSocket::bind(("127.0.0.1", 0)).expect("upstream");
        let address = upstream.local_addr().expect("address");

        let server = std::thread::spawn(move || {
            let mut buffer = [0u8; 512];
            let (read, from) = upstream.recv_from(&mut buffer).expect("query");
            assert_eq!(&buffer[..read], b"question");
            upstream.send_to(b"answer", from).expect("response");
        });

        let mut resolver = SystemResolver::new(address);
        assert_eq!(
            resolver.resolve(b"question").as_deref(),
            Some(&b"answer"[..])
        );

        server.join().expect("server");
    }
}

#[cfg(test)]
mod bridge_tests {
    use super::*;
    use dome_net::packets::{
        ETHERTYPE_IPV4, EthernetFrame, Ipv4Address, Ipv4Packet, PROTOCOL_TCP, TcpSegment, tcp_flags,
    };
    use std::io::{Read, Write};
    use std::net::TcpListener;

    const REMOTE: Ipv4Address = Ipv4Address([127, 0, 0, 1]);

    fn frame(packet: Ipv4Packet) -> Vec<u8> {
        EthernetFrame {
            destination: dome_net::nat::GATEWAY_MAC,
            source: dome_net::nat::GUEST_MAC,
            ethertype: ETHERTYPE_IPV4,
            payload: packet.to_bytes(),
        }
        .to_bytes()
    }

    fn tcp(port: u16, flags: u8, sequence: u32, payload: &[u8]) -> Vec<u8> {
        let segment = TcpSegment {
            source_port: 40000,
            destination_port: port,
            sequence,
            acknowledgement: 0,
            flags,
            window: 64240,
            payload: payload.to_vec(),
        };

        frame(Ipv4Packet {
            source: dome_net::nat::GUEST_ADDRESS,
            destination: REMOTE,
            protocol: PROTOCOL_TCP,
            ttl: 64,
            identification: 0,
            dont_fragment: true,
            payload: segment.to_bytes(dome_net::nat::GUEST_ADDRESS, REMOTE),
        })
    }

    #[test]
    fn a_request_leaves_and_the_answer_comes_back() {
        // A real server, so the test covers the socket path as well as the framing.
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("listener");
        let port = listener.local_addr().expect("address").port();

        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut request = [0u8; 5];
            stream.read_exact(&mut request).expect("request");
            assert_eq!(&request, b"GET /");
            stream
                .write_all(b"hello from the server")
                .expect("response");
            std::thread::sleep(Duration::from_millis(200));
        });

        let mut backend = UserNetBackend::new(SocketRelay::permitting());

        // Handshake, then the request.
        backend.transmit(&tcp(port, tcp_flags::SYN, 1, b""));
        let syn_ack = backend.receive().expect("the handshake is answered");
        let reply = EthernetFrame::parse(&syn_ack).expect("frame");
        let packet = Ipv4Packet::parse(&reply.payload).expect("packet");
        let segment = TcpSegment::parse(&packet.payload).expect("tcp");
        assert_eq!(segment.flags, tcp_flags::SYN | tcp_flags::ACK);

        backend.transmit(&tcp(port, tcp_flags::ACK, 2, b"GET /"));
        assert_eq!(backend.open_flows(), 1);

        // The answer arrives later, so it comes out of receive rather than out of the transmit
        // that caused it.
        let mut answer = None;
        for _ in 0..200 {
            if let Some(frame) = backend.receive() {
                let parsed = EthernetFrame::parse(&frame).expect("frame");
                let packet = Ipv4Packet::parse(&parsed.payload).expect("packet");
                let segment = TcpSegment::parse(&packet.payload).expect("tcp");
                if !segment.payload.is_empty() {
                    answer = Some(segment.payload);
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }

        assert_eq!(answer.as_deref(), Some(&b"hello from the server"[..]));
        server.join().expect("server");
    }

    #[test]
    fn an_isolated_backend_refuses_the_handshake() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("listener");
        let port = listener.local_addr().expect("address").port();

        let mut backend = UserNetBackend::new(SocketRelay::isolated());
        backend.transmit(&tcp(port, tcp_flags::SYN, 1, b""));

        let frame = backend.receive().expect("a refusal is answered");
        let parsed = EthernetFrame::parse(&frame).expect("frame");
        let packet = Ipv4Packet::parse(&parsed.payload).expect("packet");
        let segment = TcpSegment::parse(&packet.payload).expect("tcp");
        assert_eq!(segment.flags & tcp_flags::RST, tcp_flags::RST);
    }
}
