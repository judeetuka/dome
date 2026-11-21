//! Diagnostic helper: apply one cage layer and report the outcome.
//!
//! This exists because several of these layers cannot be tested from a libtest process:
//! unshare(CLONE_NEWUSER) fails with EINVAL when the process is multithreaded, and
//! seccomp/per-thread attributes are thread-local. A freshly spawned instance of this
//! program is single-threaded and gives the same guarantees the real engine will have.
//!
//! Usage: cage_probe <no-new-privs|undumpable|core|unshare|seccomp|denied-syscall>

#[allow(unsafe_code)]
fn main() {
    let mode = std::env::args().nth(1).unwrap_or_default();

    // For unshare, capture the namespaces first so the effect is provable rather than
    // inferred from a successful syscall.
    if mode == "unshare" {
        let before_net = std::fs::read_link("/proc/self/ns/net").ok();
        let before_user = std::fs::read_link("/proc/self/ns/user").ok();

        match dome_engine::sandbox::unshare_chamber_namespaces() {
            Ok(()) => {
                let after_net = std::fs::read_link("/proc/self/ns/net").ok();
                let after_user = std::fs::read_link("/proc/self/ns/user").ok();

                println!("CAGE_APPLIED");
                println!("net_changed={}", before_net != after_net);
                println!("user_changed={}", before_user != after_user);
                return;
            }
            Err(error) => {
                println!("CAGE_FAILED: {error}");
                std::process::exit(2);
            }
        }
    }

    let result = match mode.as_str() {
        "no-new-privs" => dome_engine::sandbox::no_new_privs(),
        "undumpable" => dome_engine::sandbox::make_undumpable(),
        "core" => dome_engine::sandbox::disable_core_dumps(),
        "seccomp" => dome_engine::sandbox::apply_seccomp_allowlist(),
        "denied-syscall" => {
            if let Err(error) = dome_engine::sandbox::apply_seccomp_allowlist() {
                println!("SETUP_FAILED: {error}");
                std::process::exit(2);
            }
            // reboot(2) is not in the allowlist: this must be killed by SIGSYS.
            // SAFETY: intentionally invoking a denied syscall with dummy arguments; the
            // filter is expected to terminate the process before the kernel inspects them.
            unsafe {
                libc::syscall(libc::SYS_reboot, 0, 0, 0, 0, 0);
            }
            println!("FILTER_DID_NOT_KILL");
            std::process::exit(3);
        }
        other => {
            eprintln!("unknown mode: {other}");
            std::process::exit(64);
        }
    };

    match result {
        Ok(()) => {
            println!("CAGE_APPLIED");
            report_state(&mode);
        }
        Err(error) => {
            println!("CAGE_FAILED: {error}");
            std::process::exit(2);
        }
    }
}

/// Print the observable state, so the test can assert on the effect rather than the call.
#[allow(unsafe_code)]
fn report_state(mode: &str) {
    match mode {
        "no-new-privs" | "seccomp" => {
            // Both attributes are per-thread: read the calling thread's own status.
            let status = std::fs::read_to_string("/proc/thread-self/status").unwrap_or_default();
            let value = status
                .lines()
                .find(|line| line.starts_with("NoNewPrivs:"))
                .unwrap_or("NoNewPrivs: unknown");
            println!("{value}");
        }
        "undumpable" => {
            let opened = std::fs::File::open("/proc/self/mem");
            println!("self_mem_openable={}", opened.is_ok());
        }
        "core" => {
            let limits = std::fs::read_to_string("/proc/self/limits").unwrap_or_default();
            let line = limits
                .lines()
                .find(|line| line.starts_with("Max core file size"))
                .unwrap_or("Max core file size unknown");
            println!("{line}");
        }
        _ => {}
    }
}
