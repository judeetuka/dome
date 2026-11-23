//! Building initramfs archives.
//!
//! The kernel accepts a cpio archive as the initial filesystem, which is the smallest possible
//! delivery mechanism for a foundation: no block device, no filesystem image, no partition
//! table, just files that appear at / and an init to run.
//!
//! The archive format here is the "newc" format the kernel expects. Two things about it matter
//! more than they look: every entry must be aligned to four bytes, and the archive must end with
//! a second header named TRAILER!!!, because the kernel reads records until it sees it.
//!
//! Compression is our own rather than a shelled-out gzip, so a build is reproducible on a machine
//! with no tools installed.

use std::fmt::Write as _;

use flate2::Compression;
use flate2::write::GzEncoder;

use crate::image_error::ImageError;

/// The cpio "newc" magic.
const NEWC_MAGIC: &str = "070701";

/// One file in an initramfs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Path inside the archive, without a leading slash.
    pub path: String,
    /// Contents.
    pub data: Vec<u8>,
    /// Unix mode bits.
    pub mode: u32,
}

impl Entry {
    /// A regular executable file, which is what nearly everything in an initramfs is.
    pub fn executable(path: &str, data: Vec<u8>) -> Self {
        Self {
            path: path.to_owned(),
            data,
            mode: 0o100_755,
        }
    }

    /// A regular data file.
    pub fn file(path: &str, data: Vec<u8>) -> Self {
        Self {
            path: path.to_owned(),
            data,
            mode: 0o100_644,
        }
    }

    /// A directory.
    pub fn directory(path: &str) -> Self {
        Self {
            path: path.trim_end_matches('/').to_owned(),
            data: Vec::new(),
            mode: 0o040_755,
        }
    }
}

/// An initramfs under construction.
#[derive(Debug, Default)]
pub struct Initramfs {
    entries: Vec<Entry>,
}

impl Initramfs {
    /// An empty archive.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add an entry, refusing a name that would escape the archive root.
    pub fn add(&mut self, entry: Entry) -> Result<(), ImageError> {
        let path = entry.path.trim_start_matches('/');

        if path.is_empty() {
            return Err(ImageError::Malformed(
                "initramfs entry has an empty path".to_owned(),
            ));
        }

        // A path with .. in it would place a file outside the root the kernel unpacks into, which
        // is not something a builder should be able to express by accident.
        for component in path.split('/') {
            if component == ".." {
                return Err(ImageError::Malformed(format!(
                    "initramfs path {path:?} escapes the archive root"
                )));
            }
        }

        self.entries.push(Entry {
            path: path.to_owned(),
            ..entry
        });

        Ok(())
    }

    /// An entry that is already present.
    pub fn contains(&self, path: &str) -> bool {
        let trimmed = path.trim_start_matches('/');
        self.entries.iter().any(|entry| entry.path == trimmed)
    }

    /// How many entries there are.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the archive is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The raw, uncompressed cpio archive.
    pub fn to_cpio(&self) -> Vec<u8> {
        let mut bytes = Vec::new();

        // Parents come first so the kernel can create them in order.
        for entry in order(&self.entries) {
            write_header(&mut bytes, &entry);
            bytes.extend_from_slice(&entry.data);
            pad(&mut bytes);
        }

        // The trailer is an entry with no data and the reserved name.
        write_trailer(&mut bytes);
        bytes
    }

    /// The archive as the kernel boots it.
    pub fn to_gzip(&self) -> Result<Vec<u8>, ImageError> {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        std::io::Write::write_all(&mut encoder, &self.to_cpio()).map_err(|error| {
            ImageError::Malformed(format!("cannot compress the initramfs: {error}"))
        })?;

        encoder.finish().map_err(|error| {
            ImageError::Malformed(format!("cannot compress the initramfs: {error}"))
        })
    }
}

/// Directories before the files inside them, and stable otherwise.
fn order(entries: &[Entry]) -> Vec<Entry> {
    let mut ordered: Vec<Entry> = entries.to_vec();
    ordered.sort_by(|left, right| {
        let left_depth = left.path.matches('/').count();
        let right_depth = right.path.matches('/').count();
        left_depth
            .cmp(&right_depth)
            .then_with(|| left.path.cmp(&right.path))
    });
    ordered
}

fn write_header(bytes: &mut Vec<u8>, entry: &Entry) {
    let name = entry.path.as_bytes();
    let mut header = String::with_capacity(110);

    let _ = write!(
        header,
        "{NEWC_MAGIC}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}",
        0,                // inode
        entry.mode,       // mode
        0,                // uid
        0,                // gid
        1,                // nlink
        0,                // mtime
        entry.data.len(), // filesize
        0,                // devmajor
        0,                // devminor
        0,                // rdevmajor
        0,                // rdevminor
        name.len() + 1,   // namesize, including the terminator
        0,                // checksum, unused by the kernel
    );

    bytes.extend_from_slice(header.as_bytes());
    bytes.extend_from_slice(name);
    bytes.push(0);
    pad(bytes);
}

fn write_trailer(bytes: &mut Vec<u8>) {
    let name = b"TRAILER!!!";
    let mut header = String::with_capacity(110);
    let _ = write!(
        header,
        "{NEWC_MAGIC}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}",
        0,
        0,
        0,
        0,
        1,
        0,
        0,
        0,
        0,
        0,
        0,
        name.len() + 1,
        0,
    );

    bytes.extend_from_slice(header.as_bytes());
    bytes.extend_from_slice(name);
    bytes.push(0);
    pad(bytes);
}

/// Every record starts on a four-byte boundary.
fn pad(bytes: &mut Vec<u8>) {
    while !bytes.len().is_multiple_of(4) {
        bytes.push(0);
    }
}
