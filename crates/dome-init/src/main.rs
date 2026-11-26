//! The smallest useful PID 1.
//!
//! This is the init a foundation gets when it does not ask for a service manager: mount nothing
//! it does not need, print something so a boot can be observed, and either hand over to the
//! workload or stay alive.
//!
//! It is deliberately tiny and statically linked. The kernel's own documentation lists missing
//! library dependencies as a way init fails to start, and a static binary cannot fail that way.
//! It also means a foundation containing this file needs no C library at all.

use std::io::Write;
use std::path::Path;

/// Where a workload embedded in the image is placed by the builder.
const EMBEDDED_WORKLOAD: &str = "/workload";

fn main() {
    mount_devtmpfs();

    // Anything the image needs to install for itself happens first: the installers are shell
    // scripts, so the shell has to be part of the image for this to be possible at all.
    run_bootstrap();

    // The kernel starts init with no arguments, so an embedded program is found by convention.
    let workload = std::env::args().nth(1).or_else(|| {
        Path::new(EMBEDDED_WORKLOAD)
            .is_file()
            .then(|| EMBEDDED_WORKLOAD.to_owned())
    });

    say(&format!(
        "starting as pid 1{}",
        match &workload {
            Some(path) => format!(", then handing over to {path}"),
            None => String::new(),
        }
    ));

    match workload {
        Some(path) => {
            // Replacing pid 1 rather than forking keeps the workload as the reaper, which is
            // what a single-process chamber wants.
            use std::os::unix::process::CommandExt;
            let error = std::process::Command::new(&path).arg0(&path).exec();
            say(&format!("cannot execute {path}: {error}"));
            halt(127);
        }
        None => {
            say("nothing to run, staying alive");
            // Sleeping rather than exiting: the kernel panics if pid 1 exits, and a panic would
            // look like a failure when the boot actually succeeded.
            loop {
                std::thread::sleep(std::time::Duration::from_secs(3600));
            }
        }
    }
}

/// Mount devtmpfs on /dev, which is where the console device lives.
///
/// The kernel hands init a console, but only if the device node exists. A foundation carries an
/// empty /dev, so the first thing a real init does is fill it. Without this, writes go nowhere and
/// a boot looks silent even though the binary ran.
fn mount_devtmpfs() {
    let _ = std::fs::create_dir_all("/dev");

    #[allow(unsafe_code)]
    // SAFETY: the three byte strings are NUL-terminated and outlive the call, the flags are
    // zero, and the data pointer is null, which is what devtmpfs expects for a fresh mount.
    let result = unsafe {
        libc::mount(
            c"devtmpfs".as_ptr(),
            c"/dev".as_ptr(),
            c"devtmpfs".as_ptr(),
            0,
            std::ptr::null(),
        )
    };

    // Success, or it was already mounted by a previous boot stage; either way /dev is usable.
    let _ = result;

    // /tmp and /proc are commonly expected even by a minimal workload.
    let _ = std::fs::create_dir_all("/tmp");
    let _ = std::fs::create_dir_all("/proc");
    #[allow(unsafe_code)]
    // SAFETY: as above; proc takes no data pointer.
    let _ = unsafe {
        libc::mount(
            c"proc".as_ptr(),
            c"/proc".as_ptr(),
            c"proc".as_ptr(),
            0,
            std::ptr::null(),
        )
    };
}

/// Install what the image records in /etc/dome/bootstrap.
///
/// The host is not involved: the chamber fetches each manager with its own static fetcher and
/// runs the result with its own shell.
fn run_bootstrap() {
    let Ok(manifest) = std::fs::read_to_string("/etc/dome/bootstrap") else {
        return;
    };

    let mut url: Option<String> = None;
    for line in manifest.lines() {
        match line.split_once('=') {
            Some(("url", value)) => url = Some(value.to_owned()),
            Some(("manager", name)) => {
                say(&format!("installing {name}"));
            }
            _ => {}
        }
    }

    let Some(url) = url else {
        say("no installer URL recorded; nothing to install");
        return;
    };

    say(&format!("fetching {url}"));

    let fetched = std::process::Command::new("/usr/bin/dome-fetch")
        .arg(&url)
        .arg("/run/installer")
        .status();

    match fetched {
        Ok(status) if status.success() => {}
        Ok(status) => {
            say(&format!("fetching the installer failed: {status}"));
            return;
        }
        Err(error) => {
            say(&format!("cannot run /usr/bin/dome-fetch: {error}"));
            return;
        }
    }

    // The installer is a shell script, so it runs with the shell that is part of the image.
    match std::process::Command::new("/bin/sh")
        .arg("/run/installer")
        .status()
    {
        Ok(status) => say(&format!("installer finished: {status}")),
        Err(error) => say(&format!("cannot run /bin/sh: {error}")),
    }

    let _ = std::fs::remove_file("/etc/dome/bootstrap");
}

/// Write a line where a boot observer will see it.
///
/// /dev/console is the kernel's own console and works before anything else is mounted; stdout
/// is a fallback for a run where the console device is absent.
fn say(message: &str) {
    // One-time diagnosis of the console device: if opening it fails, everything a workload prints
    // disappears without an error anywhere.
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let state = match std::fs::OpenOptions::new().write(true).open("/dev/console") {
            Ok(_) => "console-open-ok".to_owned(),
            Err(error) => format!("console-open-failed: {error}"),
        };

        if let Ok(mut kmsg) = std::fs::OpenOptions::new().write(true).open("/dev/kmsg") {
            let _ = kmsg.write_all(format!("dome-init: {state}\n").as_bytes());
        }

        let listing = std::fs::read_dir("/dev")
            .map(|entries| {
                entries
                    .flatten()
                    .filter_map(|entry| entry.file_name().into_string().ok())
                    .collect::<Vec<_>>()
                    .join(",")
            })
            .unwrap_or_else(|error| format!("unreadable: {error}"));

        if let Ok(mut kmsg) = std::fs::OpenOptions::new().write(true).open("/dev/kmsg") {
            let _ = kmsg.write_all(format!("dome-init: /dev contains {listing}\n").as_bytes());
        }
    });

    let line = format!("{message}\n");

    if let Ok(mut console) = std::fs::OpenOptions::new().write(true).open("/dev/console") {
        let _ = console.write_all(line.as_bytes());
        let _ = console.flush();
    }

    let _ = std::io::stdout().write_all(line.as_bytes());
    let _ = std::io::stdout().flush();

    // Also leave a trace on the kernel ring buffer, which survives even when the console does not.
    if let Ok(mut kmsg) = std::fs::OpenOptions::new().write(true).open("/dev/kmsg") {
        let _ = kmsg.write_all(format!("dome-init: {message}\n").as_bytes());
    }
}

fn halt(code: i32) -> ! {
    #[allow(unsafe_code)]
    // SAFETY: reboot with RB_POWER_OFF takes no pointers and does not touch memory.
    unsafe {
        libc::reboot(libc::RB_POWER_OFF);
    }
    std::process::exit(code)
}
