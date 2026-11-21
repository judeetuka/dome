//! A content-addressed store for foundations, and the writable layers chambers get from it.
//!
//! A foundation is stored once, keyed by a digest of its bytes **and its target architecture**.
//! Architecture is part of the key because the same recipe produces different bytes per target,
//! and silently serving an aarch64 image to an x86_64 chamber would fail in a way that is
//! difficult to diagnose later.
//!
//! Each chamber gets its own writable layer over the shared base. Where the filesystem supports
//! reflink (btrfs, xfs, zfs) that layer costs almost nothing on disk; elsewhere it is a plain
//! copy, and the store reports which one happened rather than implying the better case.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use dome_crypto::Sha256;

use crate::image_error::ImageError;

/// The architecture key for x86-64.
pub const ARCH_X86_64: &str = "x86_64";
/// The architecture key for 64-bit ARM.
pub const ARCH_AARCH64: &str = "aarch64";

const IMAGE: &str = "image";
const META: &str = "meta";
const REFS: &str = "refs";

/// How a chamber's writable layer was created.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum OverlayStrategy {
    /// Copy-on-write at the filesystem level: the layer shares extents with the base.
    Reflink,
    /// A full copy, because the filesystem cannot share extents.
    Copy,
}

impl OverlayStrategy {
    /// Stable name for reports.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Reflink => "reflink",
            Self::Copy => "copy",
        }
    }

    /// Whether layers from this foundation share disk with it.
    pub fn shares_disk(self) -> bool {
        self == Self::Reflink
    }
}

/// One stored foundation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundationEntry {
    /// Digest of the architecture and the bytes.
    pub digest: String,
    /// Target architecture.
    pub arch: String,
    /// Size in bytes.
    pub size: u64,
    /// The name it was inserted under, for a human reading the list.
    pub name: String,
    /// Chambers currently holding a layer.
    pub references: Vec<String>,
}

/// A chamber's writable layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Overlay {
    /// The layer's path.
    pub path: PathBuf,
    /// How it was created.
    pub strategy: OverlayStrategy,
}

/// The store.
#[derive(Debug, Clone)]
pub struct FoundationStore {
    root: PathBuf,
}

impl FoundationStore {
    /// Use a specific directory.
    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The default location under the user's data directory.
    pub fn default_root() -> PathBuf {
        if let Ok(base) = std::env::var("XDG_DATA_HOME") {
            return PathBuf::from(base).join("dome").join("store");
        }

        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_owned());
        PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("dome")
            .join("store")
    }

    /// The directory in use.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The digest a file would be stored under.
    pub fn digest_of(source: &Path, arch: &str) -> Result<String, ImageError> {
        let mut hasher = Sha256::new();
        // Architecture first, then a separator that cannot appear in an arch key, so two
        // different (arch, bytes) pairs can never collide by concatenation.
        hasher.update(arch.as_bytes());
        hasher.update(&[0]);

        let mut file = fs::File::open(source).map_err(|error| ImageError::Io {
            path: source.display().to_string(),
            source: error,
        })?;

        let mut buffer = vec![0u8; 256 * 1024];
        loop {
            let read = file.read(&mut buffer).map_err(|error| ImageError::Io {
                path: source.display().to_string(),
                source: error,
            })?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }

        Ok(hex(&hasher.finish()))
    }

    /// Store a foundation, returning the existing entry when it is already there.
    pub fn insert(
        &self,
        source: &Path,
        arch: &str,
        name: &str,
    ) -> Result<FoundationEntry, ImageError> {
        let digest = Self::digest_of(source, arch)?;
        if let Some(existing) = self.entry(&digest)? {
            return Ok(existing);
        }

        let directory = self.root.join(&digest);
        fs::create_dir_all(directory.join(REFS)).map_err(|error| ImageError::Io {
            path: directory.display().to_string(),
            source: error,
        })?;

        // Copy, then rename into place, so a half-written image is never visible under the
        // digest it claims to have.
        let temporary = directory.join("image.tmp");
        fs::copy(source, &temporary).map_err(|error| ImageError::Io {
            path: temporary.display().to_string(),
            source: error,
        })?;
        fs::rename(&temporary, directory.join(IMAGE)).map_err(|error| ImageError::Io {
            path: directory.display().to_string(),
            source: error,
        })?;

        let size = fs::metadata(directory.join(IMAGE))
            .map_err(|error| ImageError::Io {
                path: directory.display().to_string(),
                source: error,
            })?
            .len();

        write_meta(&directory, arch, size, name)?;

        self.entry(&digest)?
            .ok_or_else(|| ImageError::Malformed("the entry vanished after writing".to_owned()))
    }

    /// Read one entry.
    pub fn entry(&self, digest: &str) -> Result<Option<FoundationEntry>, ImageError> {
        let directory = self.root.join(digest);
        if !directory.join(IMAGE).is_file() {
            return Ok(None);
        }

        let (arch, size, name) = read_meta(&directory)?;
        Ok(Some(FoundationEntry {
            digest: digest.to_owned(),
            arch,
            size,
            name,
            references: references(&directory)?,
        }))
    }

    /// Every stored foundation, sorted for stable output.
    pub fn list(&self) -> Result<Vec<FoundationEntry>, ImageError> {
        let entries = match fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(ImageError::Io {
                    path: self.root.display().to_string(),
                    source: error,
                });
            }
        };

        let mut found = Vec::new();
        for entry in entries.flatten() {
            let Some(digest) = entry.file_name().to_str().map(|name| name.to_owned()) else {
                continue;
            };
            if digest.starts_with('.') || digest.contains('.') {
                continue;
            }
            if let Some(found_entry) = self.entry(&digest)? {
                found.push(found_entry);
            }
        }

        found.sort_by(|left, right| left.digest.cmp(&right.digest));
        Ok(found)
    }

    /// Give a chamber a writable layer over a foundation.
    ///
    /// The reference is recorded before the layer is returned, so a crash between the two
    /// leaves a reference to a layer nobody is using - which garbage collection reclaims -
    /// rather than a layer nobody can find.
    pub fn overlay(&self, digest: &str, chamber: &str) -> Result<Overlay, ImageError> {
        let entry = self
            .entry(digest)?
            .ok_or_else(|| ImageError::Malformed(format!("no foundation stored as {digest}")))?;
        let _ = entry;

        let directory = self.root.join(digest);
        let layers = directory.join("layers");
        fs::create_dir_all(&layers).map_err(|error| ImageError::Io {
            path: layers.display().to_string(),
            source: error,
        })?;

        let safe = sanitize(chamber);
        let path = layers.join(format!("{safe}.img"));
        let strategy = clone_or_copy(&directory.join(IMAGE), &path)?;

        fs::File::create(directory.join(REFS).join(&safe)).map_err(|error| ImageError::Io {
            path: directory.join(REFS).display().to_string(),
            source: error,
        })?;

        Ok(Overlay { path, strategy })
    }

    /// Drop a chamber's reference and its layer.
    pub fn release(&self, digest: &str, chamber: &str) -> Result<(), ImageError> {
        let directory = self.root.join(digest);
        let safe = sanitize(chamber);

        let _ = fs::remove_file(directory.join(REFS).join(&safe));
        let _ = fs::remove_file(directory.join("layers").join(format!("{safe}.img")));
        Ok(())
    }

    /// Remove foundations no chamber references, returning what was removed.
    pub fn prune(&self) -> Result<Vec<String>, ImageError> {
        let mut removed = Vec::new();

        for entry in self.list()? {
            if !entry.references.is_empty() {
                continue;
            }

            let directory = self.root.join(&entry.digest);
            fs::remove_dir_all(&directory).map_err(|error| ImageError::Io {
                path: directory.display().to_string(),
                source: error,
            })?;
            removed.push(entry.digest);
        }

        Ok(removed)
    }
}

/// Create the layer, preferring copy-on-write and reporting which happened.
fn clone_or_copy(source: &Path, destination: &Path) -> Result<OverlayStrategy, ImageError> {
    if reflink(source, destination).unwrap_or(false) {
        return Ok(OverlayStrategy::Reflink);
    }

    // A failed partial reflink leaves a file behind; start clean.
    let _ = fs::remove_file(destination);
    fs::copy(source, destination).map_err(|error| ImageError::Io {
        path: destination.display().to_string(),
        source: error,
    })?;

    Ok(OverlayStrategy::Copy)
}

/// Ask the filesystem to share extents, which only some filesystems can do.
#[allow(unsafe_code)]
fn reflink(source: &Path, destination: &Path) -> std::io::Result<bool> {
    use std::os::unix::io::AsRawFd;

    // FICLONE: share the source's extents with this destination.
    const FICLONE: libc::c_ulong = 0x4004_9409;

    let mut from = fs::File::open(source)?;
    let mut to = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)?;

    let result =
        // SAFETY: both descriptors are owned by the files just opened, and the ioctl only
        // reads the source and takes an integer argument. It touches no Rust memory.
        unsafe { libc::ioctl(to.as_raw_fd(), FICLONE, from.as_raw_fd()) };

    if result == 0 {
        return Ok(true);
    }

    let error = std::io::Error::last_os_error().raw_os_error();
    let _ = (from.flush(), to.flush());

    // Anything else is the filesystem saying it cannot share extents; the caller copies.
    match error {
        Some(libc::EOPNOTSUPP | libc::ENOTTY | libc::EXDEV | libc::EINVAL) => Ok(false),
        _ => Ok(false),
    }
}

fn write_meta(directory: &Path, arch: &str, size: u64, name: &str) -> Result<(), ImageError> {
    let path = directory.join(META);
    let body = format!("arch={arch}\nsize={size}\nname={}\n", sanitize(name));
    fs::write(&path, body).map_err(|error| ImageError::Io {
        path: path.display().to_string(),
        source: error,
    })
}

fn read_meta(directory: &Path) -> Result<(String, u64, String), ImageError> {
    let path = directory.join(META);
    let text = fs::read_to_string(&path).map_err(|error| ImageError::Io {
        path: path.display().to_string(),
        source: error,
    })?;

    let mut arch = String::new();
    let mut size = 0u64;
    let mut name = String::new();

    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key.trim() {
            "arch" => arch = value.trim().to_owned(),
            "size" => size = value.trim().parse().unwrap_or(0),
            "name" => name = value.trim().to_owned(),
            _ => {}
        }
    }

    Ok((arch, size, name))
}

fn references(directory: &Path) -> Result<Vec<String>, ImageError> {
    let path = directory.join(REFS);
    let entries = match fs::read_dir(&path) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(ImageError::Io {
                path: path.display().to_string(),
                source: error,
            });
        }
    };

    let mut names: Vec<String> = entries
        .flatten()
        .filter_map(|entry| entry.file_name().to_str().map(|name| name.to_owned()))
        .collect();
    names.sort();
    Ok(names)
}

/// Reduce a name to something safe to use as a file name.
fn sanitize(value: &str) -> String {
    value
        .chars()
        .map(|character| match character {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' => character,
            _ => '-',
        })
        .collect()
}

fn hex(bytes: &[u8; 32]) -> String {
    let mut output = String::with_capacity(64);
    for byte in bytes {
        output.push_str(&format!("{byte:02x}"));
    }
    output
}
