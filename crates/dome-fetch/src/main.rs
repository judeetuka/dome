//! Fetch a file over HTTPS, from inside a chamber.
//!
//! This exists so a chamber can install software without the host: it downloads its own package
//! manager, and that manager then installs what the recipe asked for. The host contributes no
//! binaries at all past the kernel and the init.
//!
//! Static by construction. A dynamically linked fetcher would need the host's C library inside
//! the chamber, which is the dependency this whole arrangement exists to avoid.

use std::io::Write;

fn main() {
    eprintln!("dome-fetch: starting");

    // A raw question to the resolver, so "the network is broken" and "the resolver library is
    // broken" cannot be confused: this sends the bytes itself and reports what came back.
    if std::env::args().any(|argument| argument == "--dns-probe") {
        probe_dns();
        return;
    }
    let mut args = std::env::args().skip(1);
    let (Some(url), Some(destination)) = (args.next(), args.next()) else {
        eprintln!("usage: dome-fetch <url> <destination>");
        std::process::exit(2);
    };

    match fetch(&url, &destination) {
        Ok(bytes) => println!("fetched {bytes} bytes from {url}"),
        Err(error) => {
            eprintln!("cannot fetch {url}: {error}");
            std::process::exit(1);
        }
    }
}

fn fetch(url: &str, destination: &str) -> Result<usize, String> {
    // Staged reporting on stderr: when this runs inside a chamber, "it failed" is not enough to
    // tell a name that did not resolve from a connection that was refused.
    eprintln!("dome-fetch: url {url}");

    let host = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .and_then(|rest| rest.split(['/', ':']).next())
        .unwrap_or(url);

    match std::net::ToSocketAddrs::to_socket_addrs(&(host, 443)) {
        Ok(addresses) => {
            let resolved: Vec<String> = addresses.map(|address| address.to_string()).collect();
            eprintln!("dome-fetch: resolved {host} to {}", resolved.join(", "));
        }
        Err(error) => {
            eprintln!("dome-fetch: cannot resolve {host}: {error}");
            return Err(format!("cannot resolve {host}: {error}"));
        }
    }

    eprintln!("dome-fetch: requesting");

    let response = ureq::get(url).call().map_err(|error| {
        eprintln!("dome-fetch: request failed: {error}");
        error.to_string()
    })?;

    eprintln!("dome-fetch: response received");

    let mut body = response.into_body();
    let mut reader = body.as_reader();
    let mut file = std::fs::File::create(destination)
        .map_err(|error| format!("cannot create {destination}: {error}"))?;

    let bytes = std::io::copy(&mut reader, &mut file)
        .map_err(|error| format!("cannot write {destination}: {error}"))?;

    file.flush()
        .map_err(|error| format!("cannot flush {destination}: {error}"))?;

    Ok(bytes as usize)
}

/// Ask the resolver for a name over UDP, without the resolver library.
fn probe_dns() {
    use std::net::{SocketAddr, UdpSocket};

    // The smallest useful question: one name, type A, class IN.
    let mut query = vec![
        0x12, 0x34, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];
    for label in "mise.run".split('.') {
        query.push(label.len() as u8);
        query.extend_from_slice(label.as_bytes());
    }
    query.extend_from_slice(&[0x00, 0x00, 0x01, 0x00, 0x01]);

    let socket = match UdpSocket::bind("0.0.0.0:0") {
        Ok(socket) => socket,
        Err(error) => {
            eprintln!("dns-probe: cannot bind: {error}");
            return;
        }
    };

    let _ = socket.set_read_timeout(Some(std::time::Duration::from_secs(3)));
    let resolver: SocketAddr = "10.0.2.3:53".parse().expect("resolver address");

    match socket.send_to(&query, resolver) {
        Ok(sent) => eprintln!("dns-probe: sent {sent} bytes to {resolver}"),
        Err(error) => {
            eprintln!("dns-probe: send failed: {error}");
            return;
        }
    }

    let mut answer = [0u8; 512];
    match socket.recv_from(&mut answer) {
        Ok((read, from)) => eprintln!("dns-probe: {read} bytes from {from}"),
        Err(error) => eprintln!("dns-probe: no answer: {error}"),
    }
}
