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
    dome_engine::sandbox::install_sigsys_reporter();
    let mode = std::env::args().nth(1).unwrap_or_default();

    // For unshare, capture the namespaces first so the effect is provable rather than
    // inferred from a successful syscall.
    if mode == "unshare" {
        let before_net = std::fs::read_link("/proc/self/ns/net").ok();
        let before_user = std::fs::read_link("/proc/self/ns/user").ok();

        match dome_engine::sandbox::unshare_chamber_namespaces(false) {
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
        "pid-after-fork" => {
            // The correct sequence for a PID namespace: unshare it, then fork, because the
            // calling process stays in the old namespace and cannot create threads.
            if let Err(error) = dome_engine::sandbox::unshare_chamber_namespaces(false) {
                println!("CAGE_FAILED: {error}");
                std::process::exit(2);
            }
            if let Err(error) = dome_engine::sandbox::unshare_pid_namespace() {
                println!("CAGE_FAILED: {error}");
                std::process::exit(2);
            }

            // SAFETY: fork in a single-threaded process before any KVM state exists.
            #[allow(unsafe_code)]
            let child = unsafe { libc::fork() };

            if child == 0 {
                // In the child: PID 1 of the new namespace, and threads work here.
                let spawned = std::thread::Builder::new()
                    .spawn(|| {})
                    .map(|handle| handle.join());
                println!("child_pid_one={}", std::process::id() == 1);
                println!("child_thread_ok={}", spawned.is_ok());
                // SAFETY: leaving immediately without running destructors is correct after fork.
                std::process::exit(0);
            }

            let mut status = 0;
            // SAFETY: waitpid with a valid child pid and a local status.
            unsafe { libc::waitpid(child, &mut status, 0) };
            println!("CAGE_APPLIED");
            println!(
                "parent_thread_ok={}",
                std::thread::Builder::new().spawn(|| {}).is_ok()
            );
            return;
        }
        "boot" => {
            // Boot the real kernel inside the production cage. Runs here rather than in a
            // test because the cage is irreversible: doing it in the harness would poison
            // every later test in that process.
            let kernel = std::env::args().nth(2).unwrap_or_default();
            let initramfs = std::env::args().nth(3).filter(|arg| arg != "-");

            let memory = match dome_core::MemorySize::from_mib(512) {
                Ok(memory) => memory,
                Err(error) => {
                    println!("SETUP_FAILED: {error}");
                    std::process::exit(2);
                }
            };

            let engine = match dome_engine::linux::LinuxEngine::open() {
                Ok(engine) => engine,
                Err(error) => {
                    println!("SKIP_NO_KVM: {error}");
                    return;
                }
            };

            let mut request = dome_engine::launch::LaunchRequest::new(kernel, memory);
            request.cage = true;
            request.timeout = std::time::Duration::from_secs(10);
            if let Some(initramfs) = initramfs {
                request.initramfs = Some(std::path::PathBuf::from(initramfs));
            }

            match engine.launch(&request) {
                Ok(outcome) => {
                    println!("CAGE_BOOT_OK exits={}", outcome.exits);
                    println!("CONSOLE_BEGIN");
                    print!("{}", outcome.console);
                    println!("CONSOLE_END");
                    return;
                }
                Err(error) => {
                    println!("CAGE_BOOT_FAILED: {error}");
                    std::process::exit(2);
                }
            }
        }
        "thread-steps" => {
            // Which cage layer breaks thread creation? Report after each one.
            let spawn_one = || match std::thread::Builder::new()
                .spawn(|| {})
                .map(|handle| handle.join())
            {
                Ok(_) => "ok",
                Err(_) => "FAILED",
            };

            println!("before_any={}", spawn_one());

            match dome_engine::sandbox::unshare_chamber_namespaces(false) {
                Ok(()) => println!("after_unshare={}", spawn_one()),
                Err(error) => println!("after_unshare=SKIP({error})"),
            }

            match dome_engine::sandbox::harden_engine_process() {
                Ok(()) => println!("after_harden={}", spawn_one()),
                Err(error) => println!("after_harden=SKIP({error})"),
            }

            match dome_engine::sandbox::apply_landlock(
                &dome_engine::sandbox::engine_readable_paths(None),
                &[],
            ) {
                Ok(enforcement) => {
                    println!("after_landlock={} ({})", spawn_one(), enforcement.as_str())
                }
                Err(error) => println!("after_landlock=SKIP({error})"),
            }

            match dome_engine::sandbox::apply_seccomp_allowlist() {
                Ok(()) => println!("after_seccomp={}", spawn_one()),
                Err(error) => println!("after_seccomp=SKIP({error})"),
            }

            return;
        }
        "harden-unshare" => {
            if let Err(error) = dome_engine::sandbox::harden_engine_process() {
                println!("HARDEN_FAILED: {error}");
                std::process::exit(2);
            }
            println!("hardened");
            dome_engine::sandbox::unshare_chamber_namespaces(false)
        }
        "landlock" => {
            let writable_dir = std::env::temp_dir().join("dome-landlock-sandbox");
            if let Err(error) = std::fs::create_dir_all(&writable_dir) {
                println!("SETUP_FAILED: {error}");
                std::process::exit(2);
            }

            match dome_engine::sandbox::apply_landlock(
                &dome_engine::sandbox::engine_readable_paths(None),
                std::slice::from_ref(&writable_dir),
            ) {
                Ok(enforcement) => {
                    println!("CAGE_APPLIED");
                    println!("enforcement={}", enforcement.as_str());
                    println!(
                        "read_proc={}",
                        std::fs::read_to_string("/proc/self/status").is_ok()
                    );
                    println!(
                        "write_granted={}",
                        std::fs::write(writable_dir.join("allowed.txt"), b"ok").is_ok()
                    );
                    // Nothing granted this path: with read/write rights handled, it must fail.
                    println!(
                        "write_denied={}",
                        std::fs::write("/tmp/dome-landlock-outside.txt", b"no").is_err()
                    );
                    return;
                }
                Err(error) => {
                    println!("CAGE_FAILED: {error}");
                    std::process::exit(2);
                }
            }
        }
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
