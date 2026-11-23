//! Shell profiles: where a foundation gets a shell, without a distribution.
//!
//! A chamber that installs packages needs a shell before it can install anything, because the
//! installers are scripts. That is a circular dependency unless the shell is part of the image,
//! so the builder resolves one from here.
//!
//! Only statically linked shells are accepted. A dynamic one would need the host's C library
//! inside the chamber, which is the dependency that makes an image host-specific.

use std::path::{Path, PathBuf};

/// The file a shell profile ships.
pub const SHELL_FILE: &str = "sh";

/// Where shell profiles live.
pub fn shell_root() -> PathBuf {
    if let Ok(base) = std::env::var("XDG_DATA_HOME") {
        return PathBuf::from(base).join("dome").join("shells");
    }

    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_owned());
    PathBuf::from(home)
        .join(".local")
        .join("share")
        .join("dome")
        .join("shells")
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

/// Resolve a profile to a shell binary.
pub fn resolve(root: &Path, profile: &str) -> Result<PathBuf, String> {
    let path = profile_dir(root, profile).join(SHELL_FILE);

    if path.is_file() {
        return Ok(path);
    }

    Err(format!(
        "shell profile {profile:?} is not installed: expected {}. Install one with:\n  dome shell import --profile {profile} --from <static shell>",
        path.display()
    ))
}

/// Install a statically linked shell under a profile name.
pub fn import(root: &Path, profile: &str, source: &Path) -> Result<String, String> {
    if profile.trim().is_empty() {
        return Err("a shell profile needs a name".to_owned());
    }

    let bytes = std::fs::read(source)
        .map_err(|error| format!("cannot read {}: {error}", source.display()))?;

    // Refusing here is the whole point: a dynamic shell would pull the host's C library in.
    match dome_config::installer::is_static_elf(&bytes) {
        Ok(true) => {}
        Ok(false) => {
            return Err(format!(
                "{} is dynamically linked; a shell in a chamber must be static",
                source.display()
            ));
        }
        Err(error) => return Err(format!("{}: {error}", source.display())),
    }

    let directory = profile_dir(root, profile);
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("cannot create {}: {error}", directory.display()))?;

    let target = directory.join(SHELL_FILE);
    std::fs::write(&target, &bytes)
        .map_err(|error| format!("cannot write {}: {error}", target.display()))?;

    Ok(format!(
        "installed shell profile {profile:?}\n  from: {}\n  to:   {}\n  size: {} bytes\n",
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
        .filter(|entry| entry.path().join(SHELL_FILE).is_file())
        .filter_map(|entry| entry.file_name().to_str().map(|name| name.to_owned()))
        .collect();

    profiles.sort();
    Ok(profiles)
}
