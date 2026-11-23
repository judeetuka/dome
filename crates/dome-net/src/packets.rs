//! Packets, parsed and built by hand.
//!
//! A chamber that fetches its own packages needs a network path, and the honest way to provide one
//! is to speak the protocol rather than to proxy at a higher layer. These types are the smallest
//! set that can carry a TCP connection: Ethernet, IPv4, ICMP, UDP and TCP.
//!
//! Everything here is total: a truncated or impossible packet produces an error rather than a
//! panic, because the input is whatever a guest chose to send.

use std::fmt;

/// EtherType for IPv4.
pub const ETHERTYPE_IPV4: u16 = 0x0800;
/// EtherType for ARP.
pub const ETHERTYPE_ARP: u16 = 0x0806;

/// IP protocol number for ICMP.
pub const PROTOCOL_ICMP: u8 = 1;
/// IP protocol number for TCP.
pub const PROTOCOL_TCP: u8 = 6;
/// IP protocol number for UDP.
pub const PROTOCOL_UDP: u8 = 17;

/// A packet that could not be understood.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PacketError {
    /// What was wrong.
    pub reason: String,
}

impl PacketError {
    fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

impl fmt::Display for PacketError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.reason)
    }
}

impl std::error::Error for PacketError {}

type Result<T> = std::result::Result<T, PacketError>;

/// Read a big-endian u16, or report truncation.
fn u16_at(bytes: &[u8], offset: usize) -> Result<u16> {
    let slice = bytes
        .get(offset..offset + 2)
        .ok_or_else(|| PacketError::new("truncated: wanted two bytes"))?;

    Ok(u16::from_be_bytes([slice[0], slice[1]]))
}

fn u32_at(bytes: &[u8], offset: usize) -> Result<u32> {
    let slice = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| PacketError::new("truncated: wanted four bytes"))?;

    Ok(u32::from_be_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

/// A MAC address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MacAddress(pub [u8; 6]);

impl MacAddress {
    /// The broadcast address.
    pub const BROADCAST: Self = Self([0xff; 6]);

    /// Whether this is a broadcast or multicast address.
    pub fn is_multicast(self) -> bool {
        self.0[0] & 1 == 1
    }

    /// Parse six bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let slice = bytes
            .get(..6)
            .ok_or_else(|| PacketError::new("truncated MAC address"))?;

        let mut address = [0u8; 6];
        address.copy_from_slice(slice);
        Ok(Self(address))
    }

    /// Write six bytes.
    pub fn to_bytes(self) -> [u8; 6] {
        self.0
    }
}

impl fmt::Display for MacAddress {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let parts: Vec<String> = self.0.iter().map(|byte| format!("{byte:02x}")).collect();
        formatter.write_str(&parts.join(":"))
    }
}

/// An Ethernet frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EthernetFrame {
    /// Destination.
    pub destination: MacAddress,
    /// Source.
    pub source: MacAddress,
    /// What the payload is.
    pub ethertype: u16,
    /// The payload.
    pub payload: Vec<u8>,
}

impl EthernetFrame {
    /// The header length every Ethernet II frame starts with.
    pub const HEADER_LENGTH: usize = 14;

    /// Parse a frame.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < Self::HEADER_LENGTH {
            return Err(PacketError::new("truncated Ethernet frame"));
        }

        Ok(Self {
            destination: MacAddress::from_bytes(&bytes[0..6])?,
            source: MacAddress::from_bytes(&bytes[6..12])?,
            ethertype: u16::from_be_bytes([bytes[12], bytes[13]]),
            payload: bytes[Self::HEADER_LENGTH..].to_vec(),
        })
    }

    /// Serialise a frame.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(Self::HEADER_LENGTH + self.payload.len());
        bytes.extend_from_slice(&self.destination.0);
        bytes.extend_from_slice(&self.source.0);
        bytes.extend_from_slice(&self.ethertype.to_be_bytes());
        bytes.extend_from_slice(&self.payload);
        bytes
    }
}

/// The one's complement checksum the internet uses.
///
/// Returned in network order, which is the order it is written in a header; it is not an
/// Internet checksum value to be byte-swapped again.
pub fn checksum(data: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    let mut index = 0;

    // A plain index loop rather than chunks_exact, which clippy flags for a constant chunk size
    // and whose stable replacement does not exist yet.
    while index + 1 < data.len() {
        sum += u32::from(u16::from_be_bytes([data[index], data[index + 1]]));
        index += 2;
    }

    if index < data.len() {
        sum += u32::from(u16::from_be_bytes([data[index], 0]));
    }

    while sum > 0xffff {
        sum = (sum & 0xffff) + (sum >> 16);
    }

    !(sum as u16)
}

/// A four-byte IPv4 address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Ipv4Address(pub [u8; 4]);

impl Ipv4Address {
    /// Parse four bytes.
    pub fn new(bytes: [u8; 4]) -> Self {
        Self(bytes)
    }
}

impl fmt::Display for Ipv4Address {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}.{}.{}.{}",
            self.0[0], self.0[1], self.0[2], self.0[3]
        )
    }
}

/// An IPv4 packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ipv4Packet {
    /// Source address.
    pub source: Ipv4Address,
    /// Destination address.
    pub destination: Ipv4Address,
    /// Protocol number, for example 6 for TCP.
    pub protocol: u8,
    /// Time to live.
    pub ttl: u8,
    /// Identification field.
    pub identification: u16,
    /// Don't-fragment flag.
    pub dont_fragment: bool,
    /// The payload.
    pub payload: Vec<u8>,
}

impl Ipv4Packet {
    /// Parse a packet, honouring the header length field.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 20 {
            return Err(PacketError::new("truncated IPv4 header"));
        }

        let version = bytes[0] >> 4;
        if version != 4 {
            return Err(PacketError::new(format!("not IPv4: version {version}")));
        }

        let header_length = usize::from(bytes[0] & 0x0f) * 4;
        if header_length < 20 || bytes.len() < header_length {
            return Err(PacketError::new("IPv4 header length is impossible"));
        }

        let total_length = usize::from(u16_at(bytes, 2)?);
        if total_length < header_length || total_length > bytes.len() {
            return Err(PacketError::new("IPv4 total length is impossible"));
        }

        let flags = bytes[6];
        let mut source = [0u8; 4];
        source.copy_from_slice(&bytes[12..16]);
        let mut destination = [0u8; 4];
        destination.copy_from_slice(&bytes[16..20]);

        Ok(Self {
            source: Ipv4Address(source),
            destination: Ipv4Address(destination),
            protocol: bytes[9],
            ttl: bytes[8],
            identification: u16_at(bytes, 4)?,
            dont_fragment: flags & 0x40 != 0,
            payload: bytes[header_length..total_length].to_vec(),
        })
    }

    /// Serialise a packet, computing the header checksum.
    pub fn to_bytes(&self) -> Vec<u8> {
        let total_length = 20 + self.payload.len();
        let mut bytes = Vec::with_capacity(total_length);

        bytes.push(0x45);
        bytes.push(0);
        bytes.extend_from_slice(&(total_length as u16).to_be_bytes());
        bytes.extend_from_slice(&self.identification.to_be_bytes());
        bytes.push(if self.dont_fragment { 0x40 } else { 0 });
        bytes.push(0);
        bytes.push(self.ttl);
        bytes.push(self.protocol);
        bytes.extend_from_slice(&[0, 0]);
        bytes.extend_from_slice(&self.source.0);
        bytes.extend_from_slice(&self.destination.0);

        let header_sum = checksum(&bytes);
        bytes[10..12].copy_from_slice(&header_sum.to_be_bytes());
        bytes.extend_from_slice(&self.payload);
        bytes
    }
}

/// An ICMP echo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IcmpEcho {
    /// Whether this is a request or a reply.
    pub reply: bool,
    /// Identifier chosen by the sender.
    pub identifier: u16,
    /// Sequence number.
    pub sequence: u16,
    /// Payload, echoed back verbatim.
    pub payload: Vec<u8>,
}

impl IcmpEcho {
    /// Parse an echo request or reply.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 8 {
            return Err(PacketError::new("truncated ICMP header"));
        }

        let kind = bytes[0];
        if kind != 0 && kind != 8 {
            return Err(PacketError::new(format!("not an echo: type {kind}")));
        }

        Ok(Self {
            reply: kind == 0,
            identifier: u16_at(bytes, 4)?,
            sequence: u16_at(bytes, 6)?,
            payload: bytes[8..].to_vec(),
        })
    }

    /// Serialise with a valid checksum.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(8 + self.payload.len());
        bytes.push(if self.reply { 0 } else { 8 });
        bytes.push(0);
        bytes.extend_from_slice(&[0, 0]);
        bytes.extend_from_slice(&self.identifier.to_be_bytes());
        bytes.extend_from_slice(&self.sequence.to_be_bytes());
        bytes.extend_from_slice(&self.payload);

        let sum = checksum(&bytes);
        bytes[2..4].copy_from_slice(&sum.to_be_bytes());
        bytes
    }
}

/// TCP control flags.
pub mod tcp_flags {
    /// Connection finished in this direction.
    pub const FIN: u8 = 0x01;
    /// Synchronise sequence numbers.
    pub const SYN: u8 = 0x02;
    /// Reset the connection.
    pub const RST: u8 = 0x04;
    /// Acknowledge the other side.
    pub const ACK: u8 = 0x10;
    /// Push buffered data.
    pub const PSH: u8 = 0x08;
}

/// A TCP segment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TcpSegment {
    /// Source port.
    pub source_port: u16,
    /// Destination port.
    pub destination_port: u16,
    /// Sequence number.
    pub sequence: u32,
    /// Acknowledgement number.
    pub acknowledgement: u32,
    /// Flags.
    pub flags: u8,
    /// Advertised window.
    pub window: u16,
    /// Payload.
    pub payload: Vec<u8>,
}

impl TcpSegment {
    /// Parse a segment, honouring the data offset field.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 20 {
            return Err(PacketError::new("truncated TCP header"));
        }

        let offset = usize::from(bytes[12] >> 4) * 4;
        if offset < 20 || bytes.len() < offset {
            return Err(PacketError::new("TCP data offset is impossible"));
        }

        Ok(Self {
            source_port: u16_at(bytes, 0)?,
            destination_port: u16_at(bytes, 2)?,
            sequence: u32_at(bytes, 4)?,
            acknowledgement: u32_at(bytes, 8)?,
            flags: bytes[13],
            window: u16_at(bytes, 14)?,
            payload: bytes[offset..].to_vec(),
        })
    }

    /// Serialise, computing the checksum over the pseudo-header the caller supplies.
    pub fn to_bytes(&self, source: Ipv4Address, destination: Ipv4Address) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(20 + self.payload.len());
        bytes.extend_from_slice(&self.source_port.to_be_bytes());
        bytes.extend_from_slice(&self.destination_port.to_be_bytes());
        bytes.extend_from_slice(&self.sequence.to_be_bytes());
        bytes.extend_from_slice(&self.acknowledgement.to_be_bytes());
        bytes.push(0x50);
        bytes.push(self.flags);
        bytes.extend_from_slice(&self.window.to_be_bytes());
        bytes.extend_from_slice(&[0, 0]);
        bytes.extend_from_slice(&[0, 0]);
        bytes.extend_from_slice(&self.payload);

        let sum = transport_checksum(source, destination, PROTOCOL_TCP, &bytes);
        bytes[16..18].copy_from_slice(&sum.to_be_bytes());
        bytes
    }
}

/// A UDP datagram.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UdpDatagram {
    /// Source port.
    pub source_port: u16,
    /// Destination port.
    pub destination_port: u16,
    /// Payload.
    pub payload: Vec<u8>,
}

impl UdpDatagram {
    /// Parse a datagram.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 8 {
            return Err(PacketError::new("truncated UDP header"));
        }

        let length = usize::from(u16_at(bytes, 4)?);
        if length < 8 || length > bytes.len() {
            return Err(PacketError::new("UDP length is impossible"));
        }

        Ok(Self {
            source_port: u16_at(bytes, 0)?,
            destination_port: u16_at(bytes, 2)?,
            payload: bytes[8..length].to_vec(),
        })
    }

    /// Serialise with a valid checksum.
    pub fn to_bytes(&self, source: Ipv4Address, destination: Ipv4Address) -> Vec<u8> {
        let length = 8 + self.payload.len();
        let mut bytes = Vec::with_capacity(length);
        bytes.extend_from_slice(&self.source_port.to_be_bytes());
        bytes.extend_from_slice(&self.destination_port.to_be_bytes());
        bytes.extend_from_slice(&(length as u16).to_be_bytes());
        bytes.extend_from_slice(&[0, 0]);
        bytes.extend_from_slice(&self.payload);

        let sum = transport_checksum(source, destination, PROTOCOL_UDP, &bytes);
        // A zero checksum means "not computed" in UDP, so a real zero is sent as all ones.
        let sum = if sum == 0 { 0xffff } else { sum };
        bytes[6..8].copy_from_slice(&sum.to_be_bytes());
        bytes
    }
}

/// The checksum for TCP and UDP, which cover a pseudo-header naming the endpoints.
pub fn transport_checksum(
    source: Ipv4Address,
    destination: Ipv4Address,
    protocol: u8,
    segment: &[u8],
) -> u16 {
    let mut pseudo = Vec::with_capacity(12 + segment.len());
    pseudo.extend_from_slice(&source.0);
    pseudo.extend_from_slice(&destination.0);
    pseudo.push(0);
    pseudo.push(protocol);
    pseudo.extend_from_slice(&(segment.len() as u16).to_be_bytes());
    pseudo.extend_from_slice(segment);
    checksum(&pseudo)
}
