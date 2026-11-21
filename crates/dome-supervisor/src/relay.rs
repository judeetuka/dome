//! The perimeter relay: host ports carried into a chamber's own network namespace.
//!
//! Shape forced by measurement: the supervisor cannot `setns` into the chamber's namespace,
//! and cannot create a veth pair in the host one. So the host listens, and the far end dials
//! `127.0.0.1:guest_port` **from inside the chamber**, over the socketpair created before
//! `unshare`. Every connection shares that one channel, multiplexed by [`crate::frame`].
//!
//! Two properties matter and both were bugs first:
//!
//! 1. **No data may be dropped before its sink exists.** The chamber dials asynchronously, so
//!    a data frame can arrive before the connection is established. Every connection therefore
//!    owns a queue from the moment it is announced, and a writer thread applies that queue in
//!    order. Without it small payloads vanish and large ones merely get lucky.
//! 2. **A frame's length is checked before allocation**, so a hostile header cannot decide how
//!    much memory this process commits.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

use crate::cgroup::SupervisorError;
use crate::frame::{Frame, RelayError};

/// Bytes moved per data frame.
const COPY_CHUNK: usize = 16 * 1024;

type Control = Arc<Mutex<UnixStream>>;

/// What a writer thread should do with its socket.
enum Command {
    /// Write these bytes, in the order received.
    Data(Vec<u8>),
    /// Nothing more is coming; shut the socket down.
    End,
}

/// The queue attached to one live connection.
type Sinks = Arc<Mutex<HashMap<u32, Sender<Command>>>>;

/// A published port and the address the supervisor actually bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PublishedPort {
    /// Port to reach inside the chamber.
    pub guest_port: u16,
    /// Address the supervisor listens on.
    pub bound: SocketAddr,
}

/// A running supervisor-side relay.
pub struct Relay {
    published: Vec<PublishedPort>,
    shutdown: Arc<AtomicBool>,
}

impl Relay {
    /// Where a guest port is published.
    pub fn address_of(&self, guest_port: u16) -> Option<SocketAddr> {
        self.published
            .iter()
            .find(|port| port.guest_port == guest_port)
            .map(|port| port.bound)
    }

    /// Every published port.
    pub fn published(&self) -> &[PublishedPort] {
        &self.published
    }

    /// Ask the accept loops to stop.
    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Relaxed);
    }
}

/// Start the supervisor half: bind each host port and relay it into the chamber.
pub fn spawn_supervisor(
    control: UnixStream,
    ports: &[(u16, u16)],
) -> Result<Relay, SupervisorError> {
    let control: Control = Arc::new(Mutex::new(control));
    let sinks: Sinks = Arc::new(Mutex::new(HashMap::new()));
    let shutdown = Arc::new(AtomicBool::new(false));
    let next_id = Arc::new(AtomicU32::new(1));

    let mut published = Vec::new();
    let mut listeners = Vec::new();

    for (guest_port, host_port) in ports {
        let listener = TcpListener::bind(("127.0.0.1", *host_port)).map_err(|error| {
            SupervisorError::Bind {
                port: *host_port,
                reason: error.to_string(),
            }
        })?;

        let bound = listener
            .local_addr()
            .map_err(|error| SupervisorError::Bind {
                port: *host_port,
                reason: error.to_string(),
            })?;

        published.push(PublishedPort {
            guest_port: *guest_port,
            bound,
        });
        listeners.push((*guest_port, listener));
    }

    spawn_reader(
        Arc::clone(&control),
        Arc::clone(&sinks),
        Arc::clone(&shutdown),
    );

    for (guest_port, listener) in listeners {
        let control = Arc::clone(&control);
        let sinks = Arc::clone(&sinks);
        let shutdown = Arc::clone(&shutdown);
        let next_id = Arc::clone(&next_id);

        std::thread::spawn(move || {
            for incoming in listener.incoming() {
                if shutdown.load(Ordering::Relaxed) {
                    break;
                }

                let Ok(stream) = incoming else { continue };
                let id = next_id.fetch_add(1, Ordering::Relaxed);

                let Ok(peer) = stream.try_clone() else {
                    continue;
                };

                // The queue exists before the chamber is told anything, so no byte can
                // arrive for a connection whose sink has not been created yet.
                let Some(sender) = register(&sinks, id, peer) else {
                    continue;
                };

                if send(
                    &control,
                    &Frame::Connect {
                        id,
                        port: guest_port,
                    },
                )
                .is_err()
                {
                    unregister(&sinks, id);
                    let _ = sender.send(Command::End);
                    continue;
                }

                let control = Arc::clone(&control);
                let sinks = Arc::clone(&sinks);
                std::thread::spawn(move || pump(stream, id, control, sinks));
            }
        });
    }

    Ok(Relay {
        published,
        shutdown,
    })
}

/// Start the chamber half: answer connect requests by dialling inside this namespace.
pub fn spawn_chamber(control: UnixStream) {
    let control: Control = Arc::new(Mutex::new(control));
    let sinks: Sinks = Arc::new(Mutex::new(HashMap::new()));

    std::thread::spawn(move || {
        let Some(mut reader) = lock_stream(&control) else {
            return;
        };

        loop {
            match Frame::read_from(&mut reader) {
                Ok(Some(Frame::Connect { id, port })) => {
                    // The queue is registered on the *reader* thread before any dialing
                    // happens, so a data frame that arrives during the dial is queued.
                    if let Some((sender, receiver)) = register_queued(&sinks, id) {
                        let control = Arc::clone(&control);
                        let sinks = Arc::clone(&sinks);
                        std::thread::spawn(move || {
                            dial(control, sinks, id, port, sender, receiver);
                        });
                    }
                }
                Ok(Some(frame)) => dispatch(&sinks, frame),
                Ok(None) => break,
                Err(_) => break,
            }
        }
    });
}

/// Connect inside the chamber namespace and start moving bytes both ways.
fn dial(
    control: Control,
    sinks: Sinks,
    id: u32,
    port: u16,
    sender: Sender<Command>,
    receiver: Receiver<Command>,
) {
    let stream = match TcpStream::connect(("127.0.0.1", port)) {
        Ok(stream) => stream,
        Err(_) => {
            // Nothing listens inside the chamber: say so plainly rather than hanging.
            unregister(&sinks, id);
            let _ = sender.send(Command::End);
            let _ = send(&control, &Frame::Reset { id });
            return;
        }
    };

    let Ok(mut write_half) = stream.try_clone() else {
        unregister(&sinks, id);
        let _ = send(&control, &Frame::Reset { id });
        return;
    };

    // The writer drains the queue this connection has had since it was announced, so bytes
    // that arrived during the dial are written first and in order.
    std::thread::spawn(move || {
        for command in receiver {
            match command {
                Command::Data(bytes) => {
                    if write_half.write_all(&bytes).is_err() {
                        break;
                    }
                }
                Command::End => {
                    let _ = write_half.shutdown(Shutdown::Both);
                    break;
                }
            }
        }
    });

    let _ = sender;
    pump(stream, id, control, sinks);
}

/// Copy bytes from a TCP stream into frames until it ends.
fn pump(mut stream: TcpStream, id: u32, control: Control, sinks: Sinks) {
    let mut buffer = vec![0u8; COPY_CHUNK];

    loop {
        match stream.read(&mut buffer) {
            Ok(0) => {
                let _ = send(&control, &Frame::Eof { id });
                break;
            }
            Ok(count) => {
                let frame = Frame::Data {
                    id,
                    bytes: buffer[..count].to_vec(),
                };
                if send(&control, &frame).is_err() {
                    break;
                }
            }
            Err(_) => {
                let _ = send(&control, &Frame::Reset { id });
                break;
            }
        }
    }

    unregister(&sinks, id);
}

/// Apply an inbound frame to the connection it names.
fn dispatch(sinks: &Sinks, frame: Frame) {
    match frame {
        Frame::Data { id, bytes } => {
            if let Ok(table) = sinks.lock()
                && let Some(sender) = table.get(&id)
            {
                // Ordered by construction: one queue per connection.
                let _ = sender.send(Command::Data(bytes));
            }
        }
        Frame::Eof { id } | Frame::Reset { id } => {
            if let Ok(table) = sinks.lock()
                && let Some(sender) = table.get(&id)
            {
                let _ = sender.send(Command::End);
            }
            unregister(sinks, id);
        }
        Frame::Connect { .. } => {}
    }
}

fn spawn_reader(control: Control, sinks: Sinks, shutdown: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        let Some(mut reader) = lock_stream(&control) else {
            return;
        };

        while !shutdown.load(Ordering::Relaxed) {
            match Frame::read_from(&mut reader) {
                Ok(Some(frame)) => dispatch(&sinks, frame),
                Ok(None) => break,
                Err(_) => break,
            }
        }
    });
}

/// Register a connection with a writing thread draining its queue.
fn register(sinks: &Sinks, id: u32, stream: TcpStream) -> Option<Sender<Command>> {
    let (sender, receiver) = channel::<Command>();

    if let Ok(mut table) = sinks.lock() {
        table.insert(id, sender.clone());
    } else {
        return None;
    }

    let mut writer = stream;
    std::thread::spawn(move || {
        for command in receiver {
            match command {
                Command::Data(bytes) => {
                    if writer.write_all(&bytes).is_err() {
                        break;
                    }
                }
                Command::End => {
                    let _ = writer.shutdown(Shutdown::Both);
                    break;
                }
            }
        }
    });

    Some(sender)
}

/// Register a queue without a socket yet, for a connection that is still being dialed.
///
/// The receiving end goes to the dial thread, which owns it until the connection exists. That
/// is the whole point: bytes that arrive during the dial are held in order, not dropped.
fn register_queued(sinks: &Sinks, id: u32) -> Option<(Sender<Command>, Receiver<Command>)> {
    let (sender, receiver) = channel::<Command>();

    if let Ok(mut table) = sinks.lock() {
        table.insert(id, sender.clone());
    } else {
        return None;
    }

    Some((sender, receiver))
}

fn unregister(sinks: &Sinks, id: u32) {
    if let Ok(mut table) = sinks.lock() {
        table.remove(&id);
    }
}

fn send(control: &Control, frame: &Frame) -> Result<(), RelayError> {
    let mut stream = control
        .lock()
        .map_err(|_| RelayError::Io("control channel poisoned".to_owned()))?;
    frame.write_to(&mut *stream)
}

fn lock_stream(control: &Control) -> Option<UnixStream> {
    control
        .lock()
        .ok()
        .and_then(|stream| stream.try_clone().ok())
}
