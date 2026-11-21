//! The cage around the engine process.
//!
//! ADR 0002 treats the virtualisation boundary as something that will eventually fail, so
//! these layers exist for the world after an escape: a process with no privileges, no
//! route to other processes, no core dumps, and a syscall set small enough to review.
//!
//! Nothing here is a substitute for hardware isolation. It is what is left when hardware
//! isolation loses.

use std::collections::BTreeMap;

use seccompiler::{
    BpfProgram, SeccompAction, SeccompFilter, SeccompRule, TargetArch, apply_filter,
};

use crate::EngineError;

/// Highest syscall number in the baseline block.
///
/// BASIS: Firecracker publishes its reviewed seccomp filters as JSON
/// (resources/seccomp/x86_64-unknown-linux-musl.json). Its vmm thread allows syscalls
/// 0..=97 and its vcpu thread 0..=49. That range is the informed starting point here, not
/// a guess: it is the set a production VMM was measured to need for ordinary operation.
///
/// Everything a KVM engine needs ABOVE that range is listed explicitly below, because the
/// baseline block stops short of openat/newfstatat/futex and every other modern call.
///
/// STILL OWED: a trace derived from Dome's own run (T20), replacing the baseline block
/// with exactly what this engine uses. Until then the filter is opt-in at launch.
const BASELINE_SYSCALL_CEILING: i64 = 97;

/// Syscalls the engine needs that live above the baseline block.
fn explicit_syscalls() -> &'static [libc::c_long] {
    &[
        libc::SYS_openat,
        libc::SYS_newfstatat,
        libc::SYS_futex,
        libc::SYS_clock_gettime,
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

/// Apply every layer that is safe to apply to a live process.
///
/// Order matters: no_new_privs must precede any seccomp filter, and the process should
/// drop dumpability before it maps guest memory, so the mapping is never dumpable.
pub fn harden_engine_process() -> Result<(), EngineError> {
    no_new_privs()?;
    make_undumpable()?;
    disable_core_dumps()?;
    Ok(())
}

/// Refuse to pass a setuid/setgid boundary. Required before loading a seccomp filter.
pub fn no_new_privs() -> Result<(), EngineError> {
    // SAFETY: prctl with PR_SET_NO_NEW_PRIVS takes no pointers and cannot fail in a way
    // that violates Rust's aliasing rules.
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
    // SAFETY: as above, no pointers involved.
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

/// Put the engine in its own user, mount, PID and network namespace.
///
/// A single call with all flags: unprivileged CLONE_NEWNET only succeeds when the kernel
/// creates the user namespace in the same operation (ADR 0001).
///
/// MEASURED CONSTRAINT: `unshare(CLONE_NEWUSER)` returns EINVAL if the calling process has
/// more than one thread. This must therefore be called before any thread is spawned (or
/// from a freshly forked child), which means the engine's launch sequence has to establish
/// namespaces first and start threads second. Tested via `cage_probe`: single-threaded
/// reports success, a libtest process reports EINVAL.
pub fn unshare_chamber_namespaces() -> Result<(), EngineError> {
    let flags = libc::CLONE_NEWUSER | libc::CLONE_NEWNS | libc::CLONE_NEWPID | libc::CLONE_NEWNET;

    // SAFETY: unshare takes flags by value.
    #[allow(unsafe_code)]
    let result = unsafe { libc::unshare(flags) };

    if result != 0 {
        return Err(EngineError::Setup(format!(
            "unshare failed: {}",
            std::io::Error::last_os_error()
        )));
    }

    Ok(())
}

/// Install the syscall allowlist. Everything else kills the process.
///
/// Applied last, and only when the caller asks for it: a filter that is wrong is
/// indistinguishable from a crash, so it stays opt-in until the trace-derived list lands.
pub fn apply_seccomp_allowlist() -> Result<(), EngineError> {
    no_new_privs()?;

    let mut rules: BTreeMap<i64, Vec<SeccompRule>> = BTreeMap::new();

    for syscall in allowed_syscalls() {
        // An empty rule list means "match unconditionally": seccompiler rejects a rule
        // with no conditions, so the allow is expressed by the absence of rules.
        rules.insert(syscall, Vec::new());
    }

    let filter = SeccompFilter::new(
        rules,
        SeccompAction::KillProcess,
        SeccompAction::Allow,
        TargetArch::x86_64,
    )
    .map_err(|error| EngineError::Setup(format!("seccomp filter invalid: {error}")))?;

    let program: BpfProgram = filter
        .try_into()
        .map_err(|error| EngineError::Setup(format!("seccomp compile failed: {error}")))?;

    apply_filter(&program)
        .map_err(|error| EngineError::Setup(format!("seccomp apply failed: {error}")))?;

    Ok(())
}
