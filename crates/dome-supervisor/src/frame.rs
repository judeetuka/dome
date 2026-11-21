//! Framing for the perimeter relay.
//!
//! The supervisor and the chamber speak over a socketpair created before `unshare`, because
//! nothing on the host side may enter the chamber's network namespace. Connections are
//! multiplexed over that single channel with these frames.
//!
//! Every frame is `[u8 tag][u32 length][payload]`, little endian, and no frame may exceed
//! `MAX_PAYLOAD`. A frame that is too large is refused rather than truncated: the reader is
//! the place where a hostile length would turn into an allocation.

use std::io::{Read, Write};

use thiserror::Error;

/// Largest payload a single frame may carry.
pub const MAX_PAYLOAD: usize = 64 * 1024;

/// What the two sides say to each other.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    /// Open a connection: identifier and the port to reach inside the chamber.
    Connect {
        /// Connection identifier, allocated by the supervisor.
        id: u32,
        /// Port the chamber side should connect to on 127.0.0.1.
        port: u16,
    },
    /// Bytes belonging to a connection.
    Data {
        /// Connection the bytes belong to.
        id: u32,
        /// Payload bytes.
        bytes: Vec<u8>,
    },
    /// The sender has no more bytes for this connection.
    Eof {
        /// Connection that ended cleanly.
        id: u32,
    },
    /// The connection failed and should be dropped.
    Reset {
        /// Connection that failed.
        id: u32,
    },
}

/// Why framing failed.
#[derive(Debug, Error)]
pub enum RelayError {
    /// The channel ended or refused a read.
    #[error("relay channel closed: {0}")]
    Closed(String),
    /// A frame header could not be parsed.
    #[error("malformed relay frame: {0}")]
    Malformed(String),
    /// A frame declared a payload larger than the protocol allows.
    #[error("relay frame of {length} bytes exceeds the {MAX_PAYLOAD} byte limit")]
    TooLarge {
        /// Declared payload length.
        length: usize,
    },
    /// A socket operation failed.
    #[error("relay io error: {0}")]
    Io(String),
}

const TAG_CONNECT: u8 = 1;
const TAG_DATA: u8 = 2;
const TAG_EOF: u8 = 3;
const TAG_RESET: u8 = 4;

impl Frame {
    /// Write this frame to a stream.
    pub fn write_to<W: Write>(&self, writer: &mut W) -> Result<(), RelayError> {
        let mut header = Vec::with_capacity(5);
        match self {
            Self::Connect { id, port } => {
                header.push(TAG_CONNECT);
                header.extend_from_slice(&6u32.to_le_bytes());
                header.extend_from_slice(&id.to_le_bytes());
                header.extend_from_slice(&port.to_le_bytes());
            }
            Self::Data { id, bytes } => {
                if bytes.len() > MAX_PAYLOAD {
                    return Err(RelayError::TooLarge {
                        length: bytes.len(),
                    });
                }
                header.push(TAG_DATA);
                header.extend_from_slice(&(4 + bytes.len() as u32).to_le_bytes());
                header.extend_from_slice(&id.to_le_bytes());
                header.extend_from_slice(bytes);
            }
            Self::Eof { id } => {
                header.push(TAG_EOF);
                header.extend_from_slice(&4u32.to_le_bytes());
                header.extend_from_slice(&id.to_le_bytes());
            }
            Self::Reset { id } => {
                header.push(TAG_RESET);
                header.extend_from_slice(&4u32.to_le_bytes());
                header.extend_from_slice(&id.to_le_bytes());
            }
        }

        writer
            .write_all(&header)
            .map_err(|error| RelayError::Io(error.to_string()))?;
        writer
            .flush()
            .map_err(|error| RelayError::Io(error.to_string()))
    }

    /// Read one frame. Returns `Ok(None)` when the peer closed cleanly.
    pub fn read_from<R: Read>(reader: &mut R) -> Result<Option<Self>, RelayError> {
        let mut tag = [0u8; 1];
        match reader.read(&mut tag) {
            Ok(0) => return Ok(None),
            Ok(_) => {}
            Err(error) => return Err(RelayError::Closed(error.to_string())),
        }

        let mut length_bytes = [0u8; 4];
        reader
            .read_exact(&mut length_bytes)
            .map_err(|error| RelayError::Closed(error.to_string()))?;
        let length = u32::from_le_bytes(length_bytes) as usize;

        // Check the limit before allocating: a hostile header must not decide how much
        // memory this process commits.
        if length > MAX_PAYLOAD {
            return Err(RelayError::TooLarge { length });
        }

        let mut payload = vec![0u8; length];
        reader
            .read_exact(&mut payload)
            .map_err(|error| RelayError::Closed(error.to_string()))?;

        match tag[0] {
            TAG_CONNECT => {
                if payload.len() != 6 {
                    return Err(RelayError::Malformed(format!(
                        "connect frame carried {} bytes, expected 6",
                        payload.len()
                    )));
                }
                let mut id = [0u8; 4];
                id.copy_from_slice(&payload[..4]);
                let mut port = [0u8; 2];
                port.copy_from_slice(&payload[4..]);
                Ok(Some(Self::Connect {
                    id: u32::from_le_bytes(id),
                    port: u16::from_le_bytes(port),
                }))
            }
            TAG_DATA => {
                if payload.len() < 4 {
                    return Err(RelayError::Malformed(
                        "data frame is missing its connection id".to_owned(),
                    ));
                }
                let mut id = [0u8; 4];
                id.copy_from_slice(&payload[..4]);
                Ok(Some(Self::Data {
                    id: u32::from_le_bytes(id),
                    bytes: payload[4..].to_vec(),
                }))
            }
            TAG_EOF | TAG_RESET => {
                if payload.len() != 4 {
                    return Err(RelayError::Malformed(format!(
                        "control frame carried {} bytes, expected 4",
                        payload.len()
                    )));
                }
                let mut id = [0u8; 4];
                id.copy_from_slice(&payload);
                let id = u32::from_le_bytes(id);
                Ok(Some(if tag[0] == TAG_EOF {
                    Self::Eof { id }
                } else {
                    Self::Reset { id }
                }))
            }
            other => Err(RelayError::Malformed(format!("unknown frame tag {other}"))),
        }
    }
}
