//! Kernel profiles: where a foundation gets its kernel without being told.
//!
//! A user should no more pass a kernel path than they should pass a C library: the recipe names
//! a profile, and the profile resolves to a kernel image on this host. The store exists so that
//! resolution is a lookup rather than a build, and so a machine that has built a profile once
//! never has to again.

use std::path::{Path, PathBuf};

/// The file a kernel profile ships.
pub const KERNEL_FILE: &str = "vmlinuz";

/// Where kernel profiles live.
pub fn kernel_root() -> PathBuf {
    if let Ok(base) = std::env::var("XDG_DATA_HOME") {
        return PathBuf::from(base).join("dome").join("kernels");
    }

    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_owned());
    PathBuf::from(home)
        .join(".local")
        .join("share")
        .join("dome")
        .join("kernels")
}

/// The directory a profile lives in.
pub fn profile_dir(root: &Path, profile: &str) -> PathBuf {
    let safe: String = profile
        .chars()
        .map(|character| match character {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' => character,
            _ => '-',
        })
        .collect();

    root.join(safe)
}

/// Resolve a profile to a kernel image.
pub fn resolve(root: &Path, profile: &str) -> Result<PathBuf, String> {
    let path = profile_dir(root, profile).join(KERNEL_FILE);

    if path.is_file() {
        return Ok(path);
    }

    Err(format!(
        "kernel profile {profile:?} is not installed: expected {}. Install one with: \n  dome kernel import --profile {profile} --from <bzImage>",
        path.display()
    ))
}

/// Install a kernel image under a profile name.
pub fn import(root: &Path, profile: &str, source: &Path) -> Result<String, String> {
    if profile.trim().is_empty() {
        return Err("a kernel profile needs a name".to_owned());
    }

    let bytes = std::fs::read(source)
        .map_err(|error| format!("cannot read {}: {error}", source.display()))?;

    // A kernel image starts with the boot signature; anything else is a mistake worth catching
    // here rather than at boot.
    if bytes.len() < 1024 || &bytes[510..512] != b"\x55\xaa" {
        return Err(format!(
            "{} does not look like a bootable kernel image (no boot signature)",
            source.display()
        ));
    }

    let directory = profile_dir(root, profile);
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("cannot create {}: {error}", directory.display()))?;

    let target = directory.join(KERNEL_FILE);
    std::fs::write(&target, &bytes)
        .map_err(|error| format!("cannot write {}: {error}", target.display()))?;

    Ok(format!(
        "installed kernel profile {profile:?}\n  from: {}\n  to:   {}\n  size: {} bytes\n",
        source.display(),
        target.display(),
        bytes.len()
    ))
}

/// Every installed profile, sorted.
pub fn list(root: &Path) -> Result<Vec<String>, String> {
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("cannot read {}: {error}", root.display())),
    };

    let mut profiles: Vec<String> = entries
        .flatten()
        .filter(|entry| entry.path().join(KERNEL_FILE).is_file())
        .filter_map(|entry| entry.file_name().to_str().map(|name| name.to_owned()))
        .collect();

    profiles.sort();
    Ok(profiles)
}
