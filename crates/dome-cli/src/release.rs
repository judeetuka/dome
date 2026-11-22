//! Building, inspecting and keying releases.
//!
//! The distribution default is deliberate: a release is **portable** unless asked otherwise.
//! A sealed release deliberately narrows where it can run, so making that the default would
//! surprise anyone who did not ask for it.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use dome_core::MemorySize;
use dome_crypto::pq::{suite2_generate_mldsa_key, suite2_mldsa_from_pkcs8, suite2_mldsa_to_pkcs8};
use dome_crypto::{
    generate_file_key, generate_identity, generate_signing_keypair, identity_from_string,
    recipient_from_string, signing_key_from_pkcs8, unwrap_file_key, wrap_file_key,
};
use dome_engine::launch::LaunchRequest;
use dome_engine::linux::LinuxEngine;
use dome_image::reader::ContainerVerifier;
use dome_image::seal::{
    SECTION_CHAMBER, SECTION_CREATED, SECTION_FOUNDATION, SECTION_MEMORY, SECTION_REGISTERS, Seal,
    check_resumable, distribution, read_seal, seal_is_degraded, seal_protection, write_portable,
    write_seal,
};
use dome_image::tier::{ConfidentialMechanism, HostProtection};
use dome_image::{FoundationFormat, FoundationManifest};

/// The key files a release needs.
#[derive(Debug, Clone)]
pub struct KeyFiles {
    /// Directory holding them.
    pub dir: PathBuf,
    /// age identity, for unwrapping a release file key.
    pub identity: PathBuf,
    /// age recipient, for wrapping it.
    pub recipient: PathBuf,
    /// Ed25519 signing key, PKCS#8.
    pub signing: PathBuf,
    /// Ed25519 public key.
    pub signing_public: PathBuf,
    /// ML-DSA-65 signing key, PKCS#8.
    pub mldsa: PathBuf,
    /// ML-DSA-65 public key.
    pub mldsa_public: PathBuf,
}

impl KeyFiles {
    /// Where each file lives inside a key directory.
    pub fn in_dir(dir: impl Into<PathBuf>) -> Self {
        let dir = dir.into();
        Self {
            identity: dir.join("identity.txt"),
            recipient: dir.join("recipient.txt"),
            signing: dir.join("signing.pk8"),
            signing_public: dir.join("signing.pub"),
            mldsa: dir.join("mldsa.pk8"),
            mldsa_public: dir.join("mldsa.pub"),
            dir,
        }
    }

    /// Create the directory and generate every key, refusing to overwrite.
    pub fn generate(&self) -> Result<(), String> {
        if self.identity.exists() {
            return Err(format!(
                "{} already exists; refusing to overwrite keys",
                self.identity.display()
            ));
        }

        fs::create_dir_all(&self.dir)
            .map_err(|error| format!("cannot create {}: {error}", self.dir.display()))?;

        let (secret, public) = generate_identity().map_err(|error| error.to_string())?;
        fs::write(&self.identity, format!("{secret}\n"))
            .map_err(|error| format!("cannot write identity: {error}"))?;
        fs::write(&self.recipient, format!("{public}\n"))
            .map_err(|error| format!("cannot write recipient: {error}"))?;

        let (signing, signing_public) =
            generate_signing_keypair().map_err(|error| error.to_string())?;
        fs::write(&self.signing, &signing)
            .map_err(|error| format!("cannot write signing key: {error}"))?;
        fs::write(&self.signing_public, &signing_public)
            .map_err(|error| format!("cannot write signing public key: {error}"))?;

        let (mldsa, mldsa_public) =
            suite2_generate_mldsa_key().map_err(|error| error.to_string())?;
        let mldsa_document = suite2_mldsa_to_pkcs8(&mldsa).map_err(|error| error.to_string())?;
        fs::write(&self.mldsa, &mldsa_document)
            .map_err(|error| format!("cannot write ML-DSA key: {error}"))?;
        fs::write(&self.mldsa_public, &mldsa_public)
            .map_err(|error| format!("cannot write ML-DSA public key: {error}"))?;

        Ok(())
    }

    /// Restrict the key directory to its owner.
    pub fn seal_permissions(&self) -> Result<(), String> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for path in [&self.identity, &self.signing, &self.mldsa] {
                let permissions = fs::Permissions::from_mode(0o600);
                fs::set_permissions(path, permissions)
                    .map_err(|error| format!("cannot restrict {}: {error}", path.display()))?;
            }
        }

        Ok(())
    }

    /// Read every key file. Used by packaging as well as by release.
    pub fn load(&self) -> Result<LoadedKeys, String> {
        let identity_text = read(&self.identity)?;
        let recipient_text = read(&self.recipient)?;
        let identity =
            identity_from_string(identity_text.trim()).map_err(|error| error.to_string())?;
        let recipient =
            recipient_from_string(recipient_text.trim()).map_err(|error| error.to_string())?;

        let signing_document = fs::read(&self.signing)
            .map_err(|error| format!("cannot read {}: {error}", self.signing.display()))?;
        let signing =
            signing_key_from_pkcs8(&signing_document).map_err(|error| error.to_string())?;

        let mldsa_document = fs::read(&self.mldsa)
            .map_err(|error| format!("cannot read {}: {error}", self.mldsa.display()))?;
        let mldsa = suite2_mldsa_from_pkcs8(&mldsa_document).map_err(|error| error.to_string())?;

        Ok(LoadedKeys {
            identity,
            recipient,
            signing,
            mldsa,
            signing_public: fs::read(&self.signing_public).map_err(|error| {
                format!("cannot read {}: {error}", self.signing_public.display())
            })?,
            mldsa_public: fs::read(&self.mldsa_public)
                .map_err(|error| format!("cannot read {}: {error}", self.mldsa_public.display()))?,
        })
    }
}

/// Everything a release needs, once read from disk.
pub struct LoadedKeys {
    /// age identity, for unwrapping file keys.
    pub identity: age::x25519::Identity,
    /// age recipient, for wrapping them.
    pub recipient: age::x25519::Recipient,
    /// Ed25519 signing key.
    pub signing: dome_crypto::Ed25519KeyPair,
    /// ML-DSA signing key.
    pub mldsa: dome_crypto::pq::PqdsaKeyPair,
    /// Ed25519 public key.
    pub signing_public: Vec<u8>,
    /// ML-DSA public key.
    pub mldsa_public: Vec<u8>,
}

/// Everything needed to produce a release.
#[derive(Debug, Clone)]
pub struct ReleaseOptions {
    /// Path to the cell blueprint, included in the artifact.
    pub cell: PathBuf,
    /// Kernel override. When absent, the foundation named by the cell supplies it.
    pub kernel: Option<PathBuf>,
    /// Optional initramfs override.
    pub initramfs: Option<PathBuf>,
    /// Where foundations live. Overridable so tests do not depend on the environment.
    pub foundation_root: PathBuf,
    /// Guest memory, in MiB.
    pub memory_mib: u64,
    /// Where the artifact goes.
    pub output: PathBuf,
    /// Keys to sign and wrap with.
    pub keys: KeyFiles,
    /// Seal the release, restricting it to confidential hardware.
    pub sealed: bool,
    /// The protection this device offers, as the operator declares it.
    pub captured_on: HostProtection,
    /// Wall-clock limit for the capture boot.
    pub timeout: Duration,
}

impl ReleaseOptions {
    /// Defaults: portable, a minute of boot, and this device declared unprotected.
    pub fn new(cell: impl Into<PathBuf>, output: impl Into<PathBuf>, keys: KeyFiles) -> Self {
        Self {
            cell: cell.into(),
            kernel: None,
            initramfs: None,
            foundation_root: foundation_root(),
            memory_mib: 64,
            output: output.into(),
            keys,
            sealed: false,
            captured_on: HostProtection::UNPROTECTED,
            timeout: Duration::from_secs(60),
        }
    }
}

/// Where foundations live by default.
pub fn foundation_root() -> PathBuf {
    if let Ok(base) = std::env::var("XDG_DATA_HOME") {
        return PathBuf::from(base).join("dome").join("foundations");
    }

    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_owned());
    PathBuf::from(home)
        .join(".local")
        .join("share")
        .join("dome")
        .join("foundations")
}

/// The directory holding one foundation.
///
/// The mapping from name to directory is deliberately simple and documented, because it is the
/// contract a future build step has to honour: slashes and colons become dashes, so
/// dome/buildroot:minimal lives in dome-buildroot-minimal.
pub fn foundation_dir(root: &Path, name: &str) -> PathBuf {
    let safe: String = name
        .chars()
        .map(|character| match character {
            '/' | ':' => '-',
            other => other,
        })
        .collect();

    root.join(safe)
}

const FOUNDATION_KERNEL: &str = "vmlinuz";
const FOUNDATION_INITRAMFS: &str = "initramfs.cpio.gz";

/// Find the kernel and optional initramfs to boot.
///
/// An explicit path wins. Otherwise the cell's foundation supplies them, which is how it should
/// work: the caller names an image, not the pieces inside it.
pub fn resolve_boot(
    cell_text: &str,
    kernel: Option<&Path>,
    initramfs: Option<&Path>,
    foundation_root: &Path,
) -> Result<(PathBuf, Option<PathBuf>), String> {
    let blueprint = dome_config::Blueprint::from_toml(cell_text)
        .map_err(|error| format!("the cell blueprint is not valid: {error}"))?;
    let name = blueprint.cell().foundation().as_str().to_owned();
    let directory = foundation_dir(foundation_root, &name);

    let kernel_path = match kernel {
        Some(path) => path.to_path_buf(),
        None => {
            let candidate = directory.join(FOUNDATION_KERNEL);
            if !candidate.is_file() {
                return Err(format!(
                    "foundation {name:?} is not installed: expected {} (or pass --kernel to override)",
                    candidate.display()
                ));
            }
            candidate
        }
    };

    let initramfs_path = match initramfs {
        Some(path) => Some(path.to_path_buf()),
        None => {
            let candidate = directory.join(FOUNDATION_INITRAMFS);
            candidate.is_file().then_some(candidate)
        }
    };

    Ok((kernel_path, initramfs_path))
}

/// Produce a release and report what was written.
pub fn release(options: &ReleaseOptions) -> Result<String, String> {
    let cell_text = read(&options.cell)?;
    let keys = options.keys.load()?;
    let (kernel, initramfs) = resolve_boot(
        &cell_text,
        options.kernel.as_deref(),
        options.initramfs.as_deref(),
        &options.foundation_root,
    )?;

    let engine = LinuxEngine::open().map_err(|error| format!("no usable engine: {error}"))?;
    let size = MemorySize::from_mib(options.memory_mib)
        .map_err(|error| format!("bad memory size: {error}"))?;

    let mut request = LaunchRequest::new(&kernel, size);
    request.timeout = options.timeout;
    request.capture_state = true;
    if let Some(initramfs) = initramfs.as_ref() {
        request = request.with_initramfs(initramfs);
    }

    let outcome = engine
        .launch(&request)
        .map_err(|error| format!("capture failed: {error}"))?;
    let captured = outcome
        .captured
        .ok_or_else(|| "the engine produced no state to capture".to_owned())?;

    let mut payload = Seal::new();
    payload
        .set_text(SECTION_CHAMBER, &chamber_name(&options.cell))
        .map_err(|error| error.to_string())?;
    payload
        .set_text(SECTION_CREATED, &created_unix().to_string())
        .map_err(|error| error.to_string())?;
    payload
        .set_text(
            SECTION_FOUNDATION,
            &format!("cell:{}", options.cell.display()),
        )
        .map_err(|error| error.to_string())?;
    payload
        .set_text("cell", &cell_text)
        .map_err(|error| error.to_string())?;
    payload
        .set(SECTION_MEMORY, captured.memory.clone())
        .map_err(|error| error.to_string())?;
    payload
        .set(SECTION_REGISTERS, encode_registers(&captured.registers))
        .map_err(|error| error.to_string())?;

    let manifest = FoundationManifest::new(
        dome_core::FoundationName::new("dome/release").map_err(|error| error.to_string())?,
        FoundationFormat::DomeFoundation,
        dome_core::StorageSize::from_mib(options.memory_mib).map_err(|error| error.to_string())?,
        "foundation.img",
    );

    // The file key is generated per release and wrapped to the recipient, so the artifact and
    // its key file are separate objects with separate custody.
    let file_key = generate_file_key().map_err(|error| error.to_string())?;
    let bytes = if options.sealed {
        write_seal(
            &payload,
            &manifest,
            options.captured_on,
            &file_key,
            &keys.signing,
            &keys.mldsa,
        )
    } else {
        write_portable(
            &payload,
            &manifest,
            options.captured_on,
            &file_key,
            &keys.signing,
            &keys.mldsa,
        )
    }
    .map_err(|error| format!("cannot write the release: {error}"))?;

    if let Some(parent) = options.output.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    }
    fs::write(&options.output, &bytes)
        .map_err(|error| format!("cannot write {}: {error}", options.output.display()))?;

    let wrapped = wrap_file_key(&file_key, &keys.recipient).map_err(|error| error.to_string())?;
    let key_path = key_file_for(&options.output);
    fs::write(&key_path, &wrapped)
        .map_err(|error| format!("cannot write {}: {error}", key_path.display()))?;

    let mode = if options.sealed { "sealed" } else { "portable" };
    let mut report = String::new();
    report.push_str(&format!("release: {} ({mode})\n", options.output.display()));
    report.push_str(&format!("  artifact: {} bytes\n", bytes.len()));
    report.push_str(&format!("  key: {}\n", key_path.display()));
    report.push_str(&format!(
        "  captured on: {}\n",
        options.captured_on.to_record()
    ));
    report.push_str(&format!("  kernel: {}\n", kernel.display()));
    if let Some(initramfs) = initramfs.as_ref() {
        report.push_str(&format!("  initramfs: {}\n", initramfs.display()));
    }
    if options.sealed {
        report.push_str(
            "  runs on: hardware that keeps guest memory confidential (TDX, SEV-SNP, shielded VM)\n",
        );
    } else {
        report.push_str("  runs on: any device, at whatever protection that device can give\n");
    }

    Ok(report)
}

/// Verify a release and report what it claims and whether this device can run it.
pub fn inspect(artifact: &Path, keys: &KeyFiles) -> Result<String, String> {
    let bytes = fs::read(artifact)
        .map_err(|error| format!("cannot read {}: {error}", artifact.display()))?;
    let loaded = keys.load()?;

    let wrapped = fs::read(key_file_for(artifact))
        .map_err(|error| format!("cannot read the key file: {error}"))?;
    let file_key =
        unwrap_file_key(&wrapped, &loaded.identity).map_err(|error| error.to_string())?;

    let (payload, container) = read_seal(
        &bytes,
        &file_key,
        ContainerVerifier::Hybrid {
            ed25519_public: &loaded.signing_public,
            mldsa_public: &loaded.mldsa_public,
        },
    )
    .map_err(|error| format!("cannot open the release: {error}"))?;

    let mode = distribution(&payload).map_err(|error| error.to_string())?;
    let captured_on = seal_protection(&payload).map_err(|error| error.to_string())?;
    let here = HostProtection::UNPROTECTED;

    let mut report = String::new();
    report.push_str(&format!("artifact: {}\n", artifact.display()));
    report.push_str(&format!("  distribution: {}\n", mode.as_str()));
    report.push_str(&format!("  captured on: {}\n", captured_on.to_record()));
    if seal_is_degraded(&payload) {
        report.push_str("  degraded: true (captured without hardware protection)\n");
    }
    if let Some(chamber) = payload.text(SECTION_CHAMBER) {
        report.push_str(&format!("  chamber: {chamber}\n"));
    }
    report.push_str(&format!("  memory: {} bytes\n", container.payload.len()));
    if let Some(cell) = payload.text("cell") {
        report.push_str(&format!("  cell: {} lines\n", cell.lines().count()));
    }

    match check_resumable(&payload, here) {
        Ok(effective) => {
            report.push_str(&format!(
                "  this device: runs, with {} protection\n",
                effective.tier.to_record()
            ));
        }
        Err(error) => {
            report.push_str(&format!("  this device: cannot run it ({error})\n"));
        }
    }

    Ok(report)
}

fn encode_registers(registers: &dome_engine::vcpu::VcpuSnapshot) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(44);
    for value in [
        registers.rip,
        registers.rsp,
        registers.rflags,
        registers.cr0,
        registers.cr3,
    ] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes.extend_from_slice(&registers.cs.to_le_bytes());
    bytes.extend_from_slice(&registers.ss.to_le_bytes());
    bytes
}

fn created_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|delta| delta.as_secs())
        .unwrap_or(0)
}

fn chamber_name(cell: &Path) -> String {
    cell.file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("chamber")
        .trim_end_matches(".cell")
        .to_owned()
}

/// Where the wrapped file key for an artifact lives.
pub fn key_file_for(artifact: &Path) -> PathBuf {
    let mut name = artifact
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(".key.age");
    artifact.with_file_name(name)
}

fn read(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).map_err(|error| format!("cannot read {}: {error}", path.display()))
}

/// Parse a protection tier from a command-line word.
pub fn parse_protection(value: &str) -> Option<HostProtection> {
    Some(match value {
        "none" | "unprotected" => HostProtection::UNPROTECTED,
        "tdx" => HostProtection::confidential(ConfidentialMechanism::Tdx),
        "sev-snp" => HostProtection::confidential(ConfidentialMechanism::SevSnp),
        "shielded-vm" => HostProtection::confidential(ConfidentialMechanism::ShieldedVm),
        _ => return None,
    })
}
