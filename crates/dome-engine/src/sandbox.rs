//! The cage around the engine process.
//!
//! ADR 0002 treats the virtualisation boundary as something that will eventually fail, so
//! these layers exist for the world after an escape: a process with no privileges, no route
//! to other processes, no core dumps, a filesystem it can barely see, and a syscall set
//! small enough to review.
//!
//! Nothing here is a substitute for hardware isolation. It is what is left when hardware
//! isolation loses.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use landlock::{
    AccessFs, Ruleset, RulesetAttr, RulesetCreatedAttr, RulesetStatus, path_beneath_rules,
};
use seccompiler::{
    BpfProgram, SeccompAction, SeccompFilter, SeccompRule, TargetArch, apply_filter,
};

use crate::EngineError;

/// Highest syscall number in the baseline block.
///
/// BASIS: Firecracker publishes its reviewed seccomp filters as JSON
/// (resources/seccomp/x86_64-unknown-linux-musl.json). Its vmm thread allows syscalls
/// 0..=97 and its vcpu thread 0..=49. That range is the informed starting point here, not a
/// guess, and it is validated by booting a real kernel inside the cage.
///
/// STILL OWED: a trace derived from Dome's own run (T20), replacing the baseline block with
/// exactly what this engine uses.
const BASELINE_SYSCALL_CEILING: i64 = 97;

/// Syscalls the engine needs that live above the baseline block.
fn explicit_syscalls() -> &'static [libc::c_long] {
    &[
        libc::SYS_openat,
        libc::SYS_newfstatat,
        libc::SYS_futex,
        libc::SYS_clock_gettime,
        // Measured, not guessed: 230 is what a denied boot reported first, because the watch
        // dog thread sleeps with clock_nanosleep.
        libc::SYS_clock_nanosleep,
        libc::SYS_clock_getres,
        libc::SYS_nanosleep,
        libc::SYS_getrandom,
        libc::SYS_prctl,
        libc::SYS_arch_prctl,
        libc::SYS_sigaltstack,
        libc::SYS_gettid,
        libc::SYS_tgkill,
        libc::SYS_epoll_create1,
        libc::SYS_epoll_ctl,
        libc::SYS_epoll_wait,
        libc::SYS_exit_group,
        libc::SYS_set_robust_list,
        libc::SYS_rseq,
        libc::SYS_statx,
        libc::SYS_getdents64,
        libc::SYS_restart_syscall,
        libc::SYS_sched_getaffinity,
        libc::SYS_membarrier,
        libc::SYS_prlimit64,
        libc::SYS_clone,
        libc::SYS_clone3,
    ]
}

/// Every syscall the filter permits.
fn allowed_syscalls() -> Vec<i64> {
    let mut syscalls: Vec<i64> = (0..=BASELINE_SYSCALL_CEILING).collect();

    for syscall in explicit_syscalls() {
        let number: i64 = *syscall;
        if !syscalls.contains(&number) {
            syscalls.push(number);
        }
    }

    syscalls
}

/// Refuse to cross a setuid/setgid boundary. Required before loading a seccomp filter.
pub fn no_new_privs() -> Result<(), EngineError> {
    // SAFETY: prctl with PR_SET_NO_NEW_PRIVS takes no pointers.
    #[allow(unsafe_code)]
    let result = unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) };

    if result != 0 {
        return Err(EngineError::Setup(format!(
            "PR_SET_NO_NEW_PRIVS failed: {}",
            std::io::Error::last_os_error()
        )));
    }

    Ok(())
}

/// Stop other same-uid processes from ptracing this one or reading /proc/self/mem.
pub fn make_undumpable() -> Result<(), EngineError> {
    // SAFETY: no pointers involved.
    #[allow(unsafe_code)]
    let result = unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0) };

    if result != 0 {
        return Err(EngineError::Setup(format!(
            "PR_SET_DUMPABLE failed: {}",
            std::io::Error::last_os_error()
        )));
    }

    Ok(())
}

/// Never write guest memory to disk as a core dump.
pub fn disable_core_dumps() -> Result<(), EngineError> {
    let limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };

    // SAFETY: the pointer refers to a live local for the duration of the call.
    #[allow(unsafe_code)]
    let result = unsafe { libc::setrlimit(libc::RLIMIT_CORE, &limit) };

    if result != 0 {
        return Err(EngineError::Setup(format!(
            "setrlimit(RLIMIT_CORE) failed: {}",
            std::io::Error::last_os_error()
        )));
    }

    Ok(())
}

/// Apply privilege hardening. Irreversible, and must come after the identity mapping.
pub fn harden_engine_process() -> Result<(), EngineError> {
    no_new_privs()?;
    make_undumpable()?;
    disable_core_dumps()?;
    Ok(())
}

/// Put the engine in its own user, mount and network namespace, and optionally a PID one.
///
/// A single call with all flags: unprivileged CLONE_NEWNET only succeeds when the kernel
/// creates the user namespace in the same operation (ADR 0001).
///
/// MEASURED CONSTRAINT 1: `unshare(CLONE_NEWUSER)` returns EINVAL if the calling process has
/// more than one thread, so this must run before any thread exists.
///
/// MEASURED CONSTRAINT 2: including CLONE_NEWPID makes every later `clone(CLONE_THREAD)` fail
/// with EINVAL, because threads must share the caller's PID namespace and the caller does not
/// move into the new one. A chamber must unshare the PID namespace, then fork, and only the
/// forked child may create threads. See `unshare_pid_namespace`.
///
/// MEASURED CONSTRAINT 3: the uid and gid maps must be written immediately, and before
/// `no_new_privs`, which makes /proc/self/setgroups unwritable. Without the maps the process
/// has no identity and later `clone` calls fail with EINVAL.
pub fn unshare_chamber_namespaces(include_pid_namespace: bool) -> Result<(), EngineError> {
    // SAFETY: getuid/getgid take no arguments and cannot fail.
    #[allow(unsafe_code)]
    let (uid, gid) = unsafe { (libc::getuid(), libc::getgid()) };

    let mut flags = libc::CLONE_NEWUSER | libc::CLONE_NEWNS | libc::CLONE_NEWNET;
    if include_pid_namespace {
        flags |= libc::CLONE_NEWPID;
    }

    // SAFETY: unshare takes flags by value.
    #[allow(unsafe_code)]
    let result = unsafe { libc::unshare(flags) };

    if result != 0 {
        return Err(EngineError::Setup(format!(
            "unshare failed: {}",
            std::io::Error::last_os_error()
        )));
    }

    map_identity(uid, gid)
}

/// Move future children into their own PID namespace.
///
/// Useful only immediately before a fork: the caller stays in its current PID namespace and
/// merely namespaces its children. The child becomes PID 1 there and may create threads,
/// which the caller cannot - which is exactly why this is separate.
pub fn unshare_pid_namespace() -> Result<(), EngineError> {
    // SAFETY: unshare takes flags by value.
    #[allow(unsafe_code)]
    let result = unsafe { libc::unshare(libc::CLONE_NEWPID) };

    if result != 0 {
        return Err(EngineError::Setup(format!(
            "unshare(CLONE_NEWPID) failed: {}",
            std::io::Error::last_os_error()
        )));
    }

    Ok(())
}

/// Map the current uid and gid into the new user namespace.
fn map_identity(uid: u32, gid: u32) -> Result<(), EngineError> {
    // setgroups must be denied before gid_map can be written by an unprivileged process.
    write_control("/proc/self/setgroups", "deny")?;
    write_control("/proc/self/uid_map", &format!("0 {uid} 1\n"))?;
    write_control("/proc/self/gid_map", &format!("0 {gid} 1\n"))?;

    Ok(())
}

fn write_control(path: &str, contents: &str) -> Result<(), EngineError> {
    std::fs::write(path, contents)
        .map_err(|error| EngineError::Setup(format!("cannot write {path}: {error}")))
}

/// How completely the kernel applied the filesystem restriction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LandlockEnforcement {
    /// Every requested access right is restricted.
    Full,
    /// Some rights are restricted; the rest are unavailable on this kernel.
    Partial,
    /// Landlock is unavailable or disabled here.
    None,
}

impl LandlockEnforcement {
    /// Whether anything was actually restricted.
    pub fn is_enforced(self) -> bool {
        matches!(self, Self::Full | Self::Partial)
    }

    /// Stable name for reports.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Partial => "partial",
            Self::None => "none",
        }
    }
}

/// Restrict the process to reading only the given paths, and writing only the given paths.
///
/// Handling the read rights means every path not listed becomes unreadable, which is the
/// point, but the list must cover the dynamic loader or the process cannot start its own
/// children. The restriction applies to this process and all descendants for their lifetime
/// and is not reversible.
pub fn apply_landlock(
    readable: &[PathBuf],
    writable: &[PathBuf],
) -> Result<LandlockEnforcement, EngineError> {
    let read_access = AccessFs::ReadFile | AccessFs::ReadDir | AccessFs::Execute;
    let write_access = AccessFs::WriteFile | AccessFs::MakeReg | AccessFs::RemoveFile;

    let created = Ruleset::default()
        .handle_access(read_access | write_access)
        .map_err(|error| EngineError::Setup(format!("landlock: cannot handle access: {error}")))?
        .create()
        .map_err(|error| EngineError::Setup(format!("landlock: cannot create ruleset: {error}")))?;

    // add_rules takes an iterator of Results, so the two rule sources are chained in.
    let created = created
        .add_rules(
            path_beneath_rules(readable, read_access)
                .chain(path_beneath_rules(writable, read_access | write_access)),
        )
        .map_err(|error| EngineError::Setup(format!("landlock: cannot add rules: {error}")))?;

    let status = created
        .restrict_self()
        .map_err(|error| EngineError::Setup(format!("landlock: cannot restrict self: {error}")))?;

    Ok(match status.ruleset {
        RulesetStatus::FullyEnforced => LandlockEnforcement::Full,
        RulesetStatus::PartiallyEnforced => LandlockEnforcement::Partial,
        RulesetStatus::NotEnforced => LandlockEnforcement::None,
    })
}

/// Paths an engine needs to read in order to run at all.
pub fn engine_readable_paths(kernel: Option<&Path>) -> Vec<PathBuf> {
    let mut paths = vec![
        PathBuf::from("/usr"),
        PathBuf::from("/lib"),
        PathBuf::from("/lib64"),
        PathBuf::from("/bin"),
        PathBuf::from("/etc"),
        PathBuf::from("/proc"),
        PathBuf::from("/sys/devices"),
        PathBuf::from("/dev/kvm"),
        PathBuf::from("/dev/null"),
    ];

    if let Some(kernel) = kernel {
        paths.push(kernel.to_path_buf());
    }

    paths
}

/// Report the syscall a filter denied instead of vanishing without explanation.
pub fn install_sigsys_reporter() {
    // SAFETY: sigaction writes only into a local and the handler is a plain extern fn.
    #[allow(unsafe_code)]
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = report_denied_syscall as *const () as usize;
        action.sa_flags = libc::SA_SIGINFO;
        libc::sigemptyset(&mut action.sa_mask);
        libc::sigaction(libc::SIGSYS, &action, std::ptr::null_mut());
    }
}

extern "C" fn report_denied_syscall(
    _signal: libc::c_int,
    info: *mut libc::siginfo_t,
    _context: *mut libc::c_void,
) {
    // SAFETY: the kernel supplies a valid siginfo for SIGSYS.
    #[allow(unsafe_code)]
    let number = unsafe {
        if info.is_null() {
            -1
        } else {
            (*info).si_syscall()
        }
    };

    eprintln!("DENIED_SYSCALL={number}");
    // SAFETY: exiting from a handler that cannot return is the correct action.
    #[allow(unsafe_code)]
    unsafe {
        libc::_exit(42);
    }
}

/// Install the syscall allowlist. Everything else kills the process.
pub fn apply_seccomp_allowlist() -> Result<(), EngineError> {
    install_filter(SeccompAction::KillProcess)
}

/// Install the same allowlist with a trapping denial, so a missing entry can be named.
///
/// KILL_PROCESS is the right production action and the wrong diagnostic one: the kernel kills
/// the process outright, so no handler runs and the failure looks like a silent exit. TRAP
/// delivers SIGSYS with the syscall number instead.
pub fn apply_seccomp_allowlist_trapping() -> Result<(), EngineError> {
    install_sigsys_reporter();
    install_filter(SeccompAction::Trap)
}

fn install_filter(denial: SeccompAction) -> Result<(), EngineError> {
    no_new_privs()?;

    let mut rules: BTreeMap<i64, Vec<SeccompRule>> = BTreeMap::new();

    for syscall in allowed_syscalls() {
        // An empty rule list means match unconditionally: seccompiler rejects a rule with no
        // conditions, so the allow is expressed by the absence of rules.
        rules.insert(syscall, Vec::new());
    }

    let filter = SeccompFilter::new(rules, denial, SeccompAction::Allow, TargetArch::x86_64)
        .map_err(|error| EngineError::Setup(format!("seccomp filter invalid: {error}")))?;

    let program: BpfProgram = filter
        .try_into()
        .map_err(|error| EngineError::Setup(format!("seccomp compile failed: {error}")))?;

    apply_filter(&program)
        .map_err(|error| EngineError::Setup(format!("seccomp apply failed: {error}")))?;

    Ok(())
}

/// Apply the whole cage, in the only order that works.
///
/// The ordering is entirely empirical, and each step was found by breaking it:
///
/// 1. namespaces first, because unshare(CLONE_NEWUSER) fails once a second thread exists;
/// 2. identity mapping next, because no_new_privs makes /proc/self/setgroups unwritable;
/// 3. privilege hardening, which is irreversible and would block step 2;
/// 4. Landlock, whose syscalls are deliberately absent from the allowlist;
/// 5. seccomp last, because it makes every later mistake fatal.
///
/// No PID namespace here: this process goes on to create threads, and CLONE_NEWPID would
/// make that fail. M4 adds it around a fork using `unshare_pid_namespace`.
pub fn apply_engine_cage(kernel: Option<&Path>) -> Result<LandlockEnforcement, EngineError> {
    unshare_chamber_namespaces(false)?;
    harden_engine_process()?;
    let enforcement = apply_landlock(&engine_readable_paths(kernel), &[])?;
    install_sigsys_reporter();
    apply_seccomp_allowlist()?;

    Ok(enforcement)
}

/// The same cage with trapping denials, for diagnosing a missing allowlist entry.
pub fn apply_engine_cage_diagnostic(
    kernel: Option<&Path>,
) -> Result<LandlockEnforcement, EngineError> {
    unshare_chamber_namespaces(false)?;
    harden_engine_process()?;
    let enforcement = apply_landlock(&engine_readable_paths(kernel), &[])?;
    apply_seccomp_allowlist_trapping()?;

    Ok(enforcement)
}
