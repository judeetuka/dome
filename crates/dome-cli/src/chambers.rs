//! Running, stopping and listing chambers.
//!
//! A chamber is an engine process that outlives the command that started it. The registry is
//! what lets a later command find it again, and the split here is the same one QEMU and libvirt
//! settled on years ago: the VMM does not supervise itself, something outside it keeps the
//! record.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use dome_core::{ChamberId, MemorySize};
use dome_engine::launch::LaunchRequest;
use dome_engine::linux::LinuxEngine;
use dome_supervisor::registry::{ChamberEntry, Registry};
use dome_supervisor::restart::ExitReason;

use crate::release::{KeyFiles, foundation_root, resolve_boot};

fn short_name(value: &str) -> String {
    let trimmed = value.trim_end_matches('/');
    trimmed.rsplit('/').next().unwrap_or(trimmed).to_owned()
}

/// The directory holding chamber logs.
pub fn logs_root(state_root: &Path) -> PathBuf {
    state_root.join("logs")
}

/// Options for starting a chamber.
#[derive(Debug, Clone)]
pub struct RunOptions {
    /// The blueprint.
    pub cell: PathBuf,
    /// Chamber name; defaults to the blueprint's file stem.
    pub name: Option<String>,
    /// Kernel override; the foundation supplies one otherwise.
    pub kernel: Option<PathBuf>,
    /// Initramfs override.
    pub initramfs: Option<PathBuf>,
    /// Guest memory in MiB.
    pub memory_mib: u64,
    /// CPUs the blueprint asked for.
    ///
    /// Carried so the command can refuse a request the engine cannot honour. A chamber gets one
    /// vCPU: the engine describes no CPU topology, and a kernel told it has more CPUs than exist
    /// waits forever for processors that never arrive.
    pub cpus: Option<u16>,
    /// Wall-clock limit for this chamber.
    pub timeout: Duration,
    /// Where the registry and logs live.
    pub state_root: PathBuf,
    /// Foundation search root.
    pub foundation_root: PathBuf,
    /// The program that becomes the chamber process.
    ///
    /// Defaults to this binary, whose __boot command is the chamber. It is injectable so the
    /// registry half of the loop - spawn, record, list, stop - can be tested without booting a
    /// kernel, which is a different concern from process supervision.
    pub launcher: Option<PathBuf>,
}

impl RunOptions {
    /// Defaults for a chamber started from a blueprint.
    pub fn new(cell: impl Into<PathBuf>, state_root: impl Into<PathBuf>) -> Self {
        Self {
            cell: cell.into(),
            name: None,
            kernel: None,
            initramfs: None,
            memory_mib: 64,
            cpus: None,
            timeout: Duration::from_secs(3600),
            state_root: state_root.into(),
            foundation_root: foundation_root(),
            launcher: None,
        }
    }

    /// The chamber id this would use.
    pub fn chamber_id(&self) -> Result<ChamberId, String> {
        let name = match self.name.as_deref() {
            Some(name) => name.to_owned(),
            None => short_name(&self.cell.file_stem().unwrap_or_default().to_string_lossy()),
        };

        let name = name.trim_end_matches(".cell").to_owned();
        ChamberId::new(name).map_err(|error| format!("invalid chamber name: {error}"))
    }
}

/// Start a chamber in the background and record it.
///
/// The work happens in a child process running this same binary, so the command returns while
/// the chamber keeps running. That child is the chamber.
pub fn start(options: &RunOptions) -> Result<String, String> {
    let id = options.chamber_id()?;
    // A chamber gets one vCPU: the engine describes no CPU topology to the guest, and a kernel
    // told it has more CPUs than exist waits forever for processors that never arrive. Saying so
    // is better than starting something that cannot work.
    if let Some(cpus) = options.cpus
        && cpus > 1
    {
        return Err(format!(
            "this chamber asks for {cpus} CPUs and the engine provides a single vCPU; \
             multi-CPU needs an APIC description the engine does not yet give a guest"
        ));
    }

    let registry = Registry::at(options.state_root.join("chambers"));

    let (existing, running) = match registry.get(&id).map_err(|error| error.to_string())? {
        Some(found) => found,
        None => (
            ChamberEntry {
                id: id.clone(),
                pid: None,
                cell: None,
                started_unix: 0,
                was_running: false,
            },
            false,
        ),
    };

    if running && existing.pid.is_some() {
        return Err(format!(
            "chamber {} is already running as pid {}",
            id.as_str(),
            existing.pid.unwrap_or_default()
        ));
    }

    // Fail early with a readable message rather than starting a child that dies at once. This
    // only applies to the real engine: an injected launcher has its own idea of what it needs.
    if options.launcher.is_none() {
        let cell_text = fs::read_to_string(&options.cell)
            .map_err(|error| format!("cannot read {}: {error}", options.cell.display()))?;
        resolve_boot(
            &cell_text,
            options.kernel.as_deref(),
            options.initramfs.as_deref(),
            &options.foundation_root,
        )?;
    }

    let logs = logs_root(&options.state_root);
    fs::create_dir_all(&logs)
        .map_err(|error| format!("cannot create {}: {error}", logs.display()))?;
    let log_path = logs.join(format!("{}.log", id.as_str()));
    let log = fs::File::create(&log_path)
        .map_err(|error| format!("cannot create {}: {error}", log_path.display()))?;
    let log_for_error = log
        .try_clone()
        .map_err(|error| format!("cannot duplicate the log handle: {error}"))?;

    let exe = match options.launcher.as_ref() {
        Some(path) => path.clone(),
        None => std::env::current_exe()
            .map_err(|error| format!("cannot find my own binary: {error}"))?,
    };

    let mut command = std::process::Command::new(exe);
    command
        .arg("__boot")
        .arg("--cell")
        .arg(&options.cell)
        .arg("--name")
        .arg(id.as_str())
        .arg("--memory-mib")
        .arg(options.memory_mib.to_string())
        .arg("--timeout")
        .arg(options.timeout.as_secs().to_string())
        .arg("--state")
        .arg(&options.state_root)
        .arg("--foundations")
        .arg(&options.foundation_root)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::from(log))
        .stderr(std::process::Stdio::from(log_for_error));

    if let Some(kernel) = options.kernel.as_ref() {
        command.arg("--kernel").arg(kernel);
    }
    if let Some(initramfs) = options.initramfs.as_ref() {
        command.arg("--initramfs").arg(initramfs);
    }

    let child = command
        .spawn()
        .map_err(|error| format!("cannot start the chamber: {error}"))?;
    let pid = child.id();

    registry
        .record_started(&ChamberEntry {
            id: id.clone(),
            pid: Some(pid),
            cell: Some(options.cell.display().to_string()),
            started_unix: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|delta| delta.as_secs())
                .unwrap_or(0),
            was_running: true,
        })
        .map_err(|error| format!("cannot record the chamber: {error}"))?;

    Ok(format!(
        "started {} (pid {pid})\n  log: {}\n",
        id.as_str(),
        log_path.display()
    ))
}

/// The body of a chamber process: boot, then record that it stopped.
pub fn boot(options: &RunOptions) -> Result<String, String> {
    let id = options.chamber_id()?;
    let registry = Registry::at(options.state_root.join("chambers"));

    let result = boot_inner(options);

    let reason = match &result {
        Ok(_) => ExitReason::Clean,
        Err(_) => ExitReason::Failed,
    };
    let _ = registry.record_stopped(&id, reason);

    result
}

fn boot_inner(options: &RunOptions) -> Result<String, String> {
    let cell_text = fs::read_to_string(&options.cell)
        .map_err(|error| format!("cannot read {}: {error}", options.cell.display()))?;
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
    if let Some(initramfs) = initramfs.as_ref() {
        request = request.with_initramfs(initramfs);
    }

    let outcome = engine
        .launch(&request)
        .map_err(|error| format!("the chamber failed: {error}"))?;

    Ok(outcome.console)
}

/// Stop a chamber.
pub fn stop(id: &str, state_root: &Path, grace: Duration) -> Result<String, String> {
    let chamber = ChamberId::new(id).map_err(|error| format!("invalid chamber name: {error}"))?;
    let registry = Registry::at(state_root.join("chambers"));

    let report = registry
        .stop(&chamber, grace)
        .map_err(|error| format!("cannot stop {}: {error}", chamber.as_str()))?;

    Ok(if report.already_stopped {
        format!("{} was not running", chamber.as_str())
    } else if report.forced {
        format!("{} did not stop in time and was killed", chamber.as_str())
    } else {
        format!("{} stopped", chamber.as_str())
    })
}

/// List chambers, telling the truth about which are running.
pub fn list(state_root: &Path) -> Result<String, String> {
    let registry = Registry::at(state_root.join("chambers"));
    let chambers = registry.list().map_err(|error| error.to_string())?;

    if chambers.is_empty() {
        return Ok("no chambers".to_owned());
    }

    let mut report = String::from("CHAMBER      PID      STATE    CELL\n");
    for (entry, running) in chambers {
        report.push_str(&format!(
            "{:<12} {:<8} {:<8} {}\n",
            entry.id.as_str(),
            entry
                .pid
                .map(|pid| pid.to_string())
                .unwrap_or_else(|| "-".to_owned()),
            if running { "running" } else { "stopped" },
            entry.cell.unwrap_or_else(|| "-".to_owned())
        ));
    }

    Ok(report)
}

/// The key directory convention, re-exported so the CLI has one source for it.
pub fn keys_dir(path: &Path) -> KeyFiles {
    KeyFiles::in_dir(path)
}
