//! Relay tests.
//!
//! The in-process tests stand a plain TCP echo server in for the chamber's own service and
//! run both halves of the relay over a socketpair, which exercises framing, multiplexing,
//! back-pressure and teardown. What they cannot prove is the namespace boundary - that is
//! what T23's separate netns test covers.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::net::UnixStream;
use std::time::Duration;

use dome_supervisor::frame::{Frame, MAX_PAYLOAD, RelayError};
use dome_supervisor::relay::{Relay, spawn_chamber, spawn_supervisor};

/// A TCP echo server standing in for whatever listens inside the chamber.
fn echo_server() -> (u16, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind echo");
    let port = listener.local_addr().expect("addr").port();

    let handle = std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            std::thread::spawn(move || {
                let mut buffer = [0u8; 4096];
                while let Ok(count) = stream.read(&mut buffer) {
                    if count == 0 {
                        break;
                    }
                    if stream.write_all(&buffer[..count]).is_err() {
                        break;
                    }
                }
            });
        }
    });

    (port, handle)
}

fn start_relay(guest_port: u16) -> Relay {
    let (supervisor_end, chamber_end) = UnixStream::pair().expect("socketpair");
    let relay = spawn_supervisor(supervisor_end, &[(guest_port, 0)]).expect("spawn relay");
    spawn_chamber(chamber_end);
    relay
}

fn read_with_timeout(stream: &mut TcpStream, expected: usize) -> Vec<u8> {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("timeout");

    let mut buffer = vec![0u8; expected];
    let mut filled = 0;
    while filled < expected {
        match stream.read(&mut buffer[filled..]) {
            Ok(0) => break,
            Ok(count) => filled += count,
            Err(_) => break,
        }
    }
    buffer.truncate(filled);
    buffer
}

#[test]
fn a_byte_travels_host_to_chamber_and_back() {
    let (guest_port, _echo) = echo_server();
    let relay = start_relay(guest_port);

    let address = relay.address_of(guest_port).expect("published address");
    let mut client = TcpStream::connect(address).expect("connect to published port");
    client.write_all(b"hello chamber").expect("write");
    client.flush().expect("flush");

    let echoed = read_with_timeout(&mut client, 13);
    assert_eq!(echoed, b"hello chamber");

    relay.shutdown();
}

#[test]
fn a_larger_payload_survives_the_frame_boundary() {
    let (guest_port, _echo) = echo_server();
    let relay = start_relay(guest_port);

    // Larger than one COPY_CHUNK, so it must be split across data frames and reassembled.
    let payload: Vec<u8> = (0..70_000).map(|i| (i % 251) as u8).collect();

    let address = relay.address_of(guest_port).expect("published address");
    let mut client = TcpStream::connect(address).expect("connect");

    // Two handles on one socket: one writes while the other reads, which is what a real
    // TCP client does and what avoids a deadlock when the payload exceeds socket buffers.
    let mut writer = client.try_clone().expect("clone");
    let written = payload.clone();
    std::thread::spawn(move || {
        let _ = writer.write_all(&written);
    });

    let received = read_with_timeout(&mut client, payload.len());
    assert_eq!(received.len(), payload.len(), "relay truncated the payload");
    assert_eq!(received, payload);

    relay.shutdown();
}

#[test]
fn several_connections_are_multiplexed() {
    let (guest_port, _echo) = echo_server();
    let relay = start_relay(guest_port);
    let address = relay.address_of(guest_port).expect("published address");

    let mut handles = Vec::new();
    for index in 0..8u8 {
        handles.push(std::thread::spawn(move || {
            let mut client = TcpStream::connect(address).expect("connect");
            let message = vec![index; 1024];
            client.write_all(&message).expect("write");
            client.flush().expect("flush");

            let echoed = read_with_timeout(&mut client, 1024);
            assert_eq!(echoed, message, "connection {index} got the wrong bytes");
        }));
    }

    for handle in handles {
        handle.join().expect("connection thread");
    }

    relay.shutdown();
}

#[test]
fn a_port_nothing_listens_on_closes_rather_than_hanging() {
    // Publish a guest port where the chamber has no listener: the client must see the
    // connection end, not sit in a read that never returns.
    let dead_port = {
        let probe = TcpListener::bind("127.0.0.1:0").expect("probe");
        probe.local_addr().expect("addr").port()
    };

    let relay = start_relay(dead_port);
    let address = relay.address_of(dead_port).expect("published address");

    let mut client = TcpStream::connect(address).expect("connect");
    client.write_all(b"anyone there?").expect("write");

    let echoed = read_with_timeout(&mut client, 16);
    assert!(
        echoed.is_empty(),
        "expected the connection to close, got {echoed:?}"
    );

    relay.shutdown();
}

// --- frame codec ---

#[test]
fn every_frame_kind_round_trips() {
    let frames = vec![
        Frame::Connect { id: 7, port: 8080 },
        Frame::Data {
            id: 7,
            bytes: vec![1, 2, 3, 4],
        },
        Frame::Eof { id: 7 },
        Frame::Reset { id: 9 },
    ];

    let mut buffer = Vec::new();
    for frame in &frames {
        frame.write_to(&mut buffer).expect("write frame");
    }

    let mut cursor = std::io::Cursor::new(buffer);
    for expected in &frames {
        let decoded = Frame::read_from(&mut cursor)
            .expect("read frame")
            .expect("some frame");
        assert_eq!(&decoded, expected);
    }

    // Clean end of stream is None, not an error.
    assert!(Frame::read_from(&mut cursor).expect("eof").is_none());
}

#[test]
fn an_oversized_length_is_refused_before_allocating() {
    // A hostile header must not decide how much memory this process commits.
    let mut hostile = vec![2u8];
    hostile.extend_from_slice(&(MAX_PAYLOAD as u32 + 1).to_le_bytes());
    hostile.extend_from_slice(&[0u8; 8]);

    match Frame::read_from(&mut std::io::Cursor::new(hostile)) {
        Err(RelayError::TooLarge { length }) => assert_eq!(length, MAX_PAYLOAD + 1),
        other => panic!("expected TooLarge, got {other:?}"),
    }
}

#[test]
fn an_unknown_tag_is_malformed_rather_than_ignored() {
    let mut frame = vec![99u8];
    frame.extend_from_slice(&0u32.to_le_bytes());

    match Frame::read_from(&mut std::io::Cursor::new(frame)) {
        Err(RelayError::Malformed(_)) => {}
        other => panic!("expected Malformed, got {other:?}"),
    }
}

#[test]
fn a_truncated_frame_is_reported() {
    let mut buffer = Vec::new();
    Frame::Data {
        id: 1,
        bytes: vec![0u8; 32],
    }
    .write_to(&mut buffer)
    .expect("write");

    buffer.truncate(buffer.len() - 4);

    match Frame::read_from(&mut std::io::Cursor::new(buffer)) {
        Err(RelayError::Closed(_)) => {}
        other => panic!("expected Closed, got {other:?}"),
    }
}
