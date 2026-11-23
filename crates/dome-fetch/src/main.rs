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
    let response = ureq::get(url).call().map_err(|error| error.to_string())?;

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
