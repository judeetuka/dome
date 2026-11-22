//! Packaging: turning an artifact into a portable export, and back.
//!
//! An export is self-contained on purpose. The seal inside it is encrypted with its own file
//! key, so the export carries that key too, wrapped to the recipient. Without it an export could
//! only ever be imported on the machine that produced it, which defeats the point.

use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use dome_crypto::{generate_file_key, unwrap_file_key, wrap_file_key};
use dome_image::export::{Export, ExportMeta, read_export, write_export};
use dome_image::reader::ContainerVerifier;
use dome_image::seal::{
    SECTION_FOUNDATION, check_resumable, distribution, read_seal, seal_is_degraded, seal_protection,
};
use dome_image::tier::HostProtection;

use crate::release::{KeyFiles, key_file_for};

const SECTION_CELL: &str = "cell";

/// Package an artifact as an export.
pub fn export(source: &Path, keys: &KeyFiles, output: &Path) -> Result<String, String> {
    let bytes =
        fs::read(source).map_err(|error| format!("cannot read {}: {error}", source.display()))?;
    let wrapped = fs::read(key_file_for(source))
        .map_err(|error| format!("cannot read the artifact key: {error}"))?;

    let loaded = keys.load()?;
    let file_key =
        unwrap_file_key(&wrapped, &loaded.identity).map_err(|error| error.to_string())?;

    let (payload, _) = read_seal(
        &bytes,
        &file_key,
        ContainerVerifier::Hybrid {
            ed25519_public: &loaded.signing_public,
            mldsa_public: &loaded.mldsa_public,
        },
    )
    .map_err(|error| format!("cannot open the artifact: {error}"))?;

    let cell = payload
        .text(SECTION_CELL)
        .ok_or_else(|| "the artifact carries no cell blueprint".to_owned())?;
    let foundation = payload
        .text(SECTION_FOUNDATION)
        .unwrap_or_else(|| "unknown".to_owned());
    let seal_tier = seal_protection(&payload).map_err(|error| error.to_string())?;

    let meta = ExportMeta::new(foundation, created_unix())
        .on_host(seal_tier)
        .with_seal_protection(seal_tier);

    let export = Export::with_meta(&cell, Some(bytes.clone()), Some(wrapped.clone()), "", meta)
        .map_err(|error| format!("cannot package the export: {error}"))?;

    // The package gets its own file key, so its confidentiality does not depend on the
    // artifact's key surviving.
    let export_key = generate_file_key().map_err(|error| error.to_string())?;
    let manifest = dome_image::FoundationManifest::new(
        dome_core::FoundationName::new("dome/export").map_err(|error| error.to_string())?,
        dome_image::FoundationFormat::DomeFoundation,
        dome_core::StorageSize::from_mib(64).map_err(|error| error.to_string())?,
        "foundation.img",
    );

    let packed = write_export(
        &export,
        &manifest,
        &export_key,
        &loaded.signing,
        &loaded.mldsa,
    )
    .map_err(|error| format!("cannot write the export: {error}"))?;

    fs::write(output, &packed)
        .map_err(|error| format!("cannot write {}: {error}", output.display()))?;

    let wrapped_key =
        wrap_file_key(&export_key, &loaded.recipient).map_err(|error| error.to_string())?;
    let key_path = key_file_for(output);
    fs::write(&key_path, &wrapped_key)
        .map_err(|error| format!("cannot write {}: {error}", key_path.display()))?;

    let mode = distribution(&payload).map_err(|error| error.to_string())?;
    let mut report = String::new();
    report.push_str(&format!(
        "exported {} -> {}\n",
        source.display(),
        output.display()
    ));
    report.push_str(&format!("  distribution: {}\n", mode.as_str()));
    report.push_str(&format!("  captured on: {}\n", seal_tier.to_record()));
    if seal_is_degraded(&payload) {
        report.push_str("  degraded: true (captured without hardware protection)\n");
    }
    report.push_str(&format!("  key: {}\n", key_path.display()));
    report.push_str(&format!("  size: {} bytes\n", packed.len()));

    Ok(report)
}

/// Unpack an export back into a usable artifact.
pub fn import(source: &Path, keys: &KeyFiles, output: &Path) -> Result<String, String> {
    let bytes =
        fs::read(source).map_err(|error| format!("cannot read {}: {error}", source.display()))?;
    let wrapped = fs::read(key_file_for(source))
        .map_err(|error| format!("cannot read the export key: {error}"))?;

    let loaded = keys.load()?;
    let file_key =
        unwrap_file_key(&wrapped, &loaded.identity).map_err(|error| error.to_string())?;

    let (export, _) = read_export(
        &bytes,
        &file_key,
        ContainerVerifier::Hybrid {
            ed25519_public: &loaded.signing_public,
            mldsa_public: &loaded.mldsa_public,
        },
    )
    .map_err(|error| format!("cannot open the export: {error}"))?;

    let seal = export
        .seal()
        .ok_or_else(|| "the export carries no chamber state".to_owned())?;
    let seal_key = export
        .wrapped_key()
        .ok_or_else(|| "the export carries no key for its chamber state".to_owned())?;

    fs::write(output, seal)
        .map_err(|error| format!("cannot write {}: {error}", output.display()))?;
    let key_path = key_file_for(output);
    fs::write(&key_path, seal_key)
        .map_err(|error| format!("cannot write {}: {error}", key_path.display()))?;

    // The imported artifact must be openable with this key directory, or the import is a
    // file nobody can use.
    let (payload, _) = read_seal(
        &fs::read(output).map_err(|error| format!("cannot read back: {error}"))?,
        &unwrap_file_key(seal_key, &loaded.identity).map_err(|error| error.to_string())?,
        ContainerVerifier::Hybrid {
            ed25519_public: &loaded.signing_public,
            mldsa_public: &loaded.mldsa_public,
        },
    )
    .map_err(|error| format!("the imported artifact does not open: {error}"))?;

    let mode = distribution(&payload).map_err(|error| error.to_string())?;
    let runnable = match check_resumable(&payload, HostProtection::UNPROTECTED) {
        Ok(effective) => format!("runs here, with {} protection", effective.tier.to_record()),
        Err(error) => format!("cannot run here: {error}"),
    };

    Ok(format!(
        "imported {} -> {}\n  distribution: {}\n  this device: {runnable}\n  key: {}\n",
        source.display(),
        output.display(),
        mode.as_str(),
        key_path.display()
    ))
}

fn created_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|delta| delta.as_secs())
        .unwrap_or(0)
}
