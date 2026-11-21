//! Cage tests.
//!
//! Each layer is exercised through the `cage_probe` binary rather than in-process.
//! Two measured reasons: unshare(CLONE_NEWUSER) returns EINVAL when the caller is
//! multithreaded (libtest processes are), and NoNewPrivs/seccomp are per-thread so the
//! harness thread is the wrong place to observe them.

use std::process::{Command, Output};

fn probe(mode: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cage_probe"))
        .arg(mode)
        .output()
        .expect("spawn cage_probe")
}

fn text_of(output: &Output) -> String {
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    text
}

#[test]
fn no_new_privs_is_set() {
    let output = probe("no-new-privs");
    let text = text_of(&output);

    assert!(output.status.success(), "probe failed: {text}");
    assert!(text.contains("NoNewPrivs:\t1"), "not set: {text}");
}

#[test]
fn process_cannot_be_dumped() {
    let output = probe("undumpable");
    let text = text_of(&output);

    assert!(output.status.success(), "probe failed: {text}");
    // With dumpability cleared, /proc/self/mem is no longer openable by the process itself.
    assert!(
        text.contains("self_mem_openable=false"),
        "still dumpable: {text}"
    );
}

#[test]
fn core_dumps_are_disabled() {
    let output = probe("core");
    let text = text_of(&output);

    assert!(output.status.success(), "probe failed: {text}");
    assert!(
        text.contains("Max core file size"),
        "no core limit line: {text}"
    );
    let line = text
        .lines()
        .find(|l| l.starts_with("Max core file size"))
        .unwrap_or("");
    let fields: Vec<&str> = line.split_whitespace().collect();
    assert_eq!(
        fields.get(4).copied(),
        Some("0"),
        "core limit not zero: {line}"
    );
}

#[test]
fn namespaces_change() {
    let output = probe("unshare");
    let text = text_of(&output);

    assert!(output.status.success(), "probe failed: {text}");
    assert!(
        text.contains("net_changed=true"),
        "net ns unchanged: {text}"
    );
    assert!(
        text.contains("user_changed=true"),
        "user ns unchanged: {text}"
    );
}

#[test]
fn denied_syscall_kills_the_process() {
    let output = probe("denied-syscall");

    // Killed by SIGSYS (31): not a clean exit, and not a panic.
    use std::os::unix::process::ExitStatusExt;
    assert_eq!(
        output.status.signal(),
        Some(31),
        "expected SIGSYS, got {:?}: {}",
        output.status,
        text_of(&output)
    );
}

#[test]
fn the_allowlist_still_permits_normal_work() {
    // A filter that kills the process for doing ordinary work is worse than no filter.
    let output = probe("seccomp");
    let text = text_of(&output);

    assert!(
        output.status.success(),
        "probe died under its own filter: {text}"
    );
    assert!(text.contains("CAGE_APPLIED"), "filter not applied: {text}");
}
