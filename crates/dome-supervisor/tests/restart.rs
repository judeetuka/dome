//! Restart policy rules and durable records.
//!
//! The policy matrix is asserted in full because the interesting cases are the ones that are
//! easy to get subtly wrong: a clean exit under `always`, a manual stop under any policy, and
//! a bounded retry budget that has run out.

use dome_core::ChamberId;
use dome_supervisor::restart::{ExitReason, RestartPolicy, RestartState};
use dome_supervisor::state::{ChamberStore, PersistedChamber};

fn id(name: &str) -> ChamberId {
    ChamberId::new(name).expect("chamber id")
}

fn record(
    name: &str,
    policy: RestartPolicy,
    reason: ExitReason,
    was_running: bool,
) -> PersistedChamber {
    PersistedChamber {
        id: id(name),
        policy,
        state: RestartState {
            attempts: 0,
            reason,
        },
        was_running,
        pid: None,
        cell: None,
        started_unix: 0,
    }
}

#[test]
fn no_policy_never_restarts() {
    for reason in [ExitReason::Clean, ExitReason::Failed, ExitReason::HostLost] {
        assert!(!RestartPolicy::No.should_restart(reason, 0), "{reason:?}");
    }
}

#[test]
fn an_operator_stop_always_sticks() {
    // Every policy, including always. This is the flaw not copied from Docker.
    for policy in [
        RestartPolicy::No,
        RestartPolicy::OnFailure { max_retries: None },
        RestartPolicy::Always,
        RestartPolicy::UnlessStopped,
    ] {
        assert!(
            !policy.should_restart(ExitReason::StoppedByOperator, 0),
            "{policy:?} restarted after a deliberate stop"
        );
    }
}

#[test]
fn always_restarts_after_a_clean_exit_or_a_lost_host() {
    assert!(RestartPolicy::Always.should_restart(ExitReason::Clean, 0));
    assert!(RestartPolicy::Always.should_restart(ExitReason::HostLost, 0));
}

#[test]
fn unless_stopped_is_identical_to_always_by_design() {
    // Docker's distinction was that `always` overrides a manual stop; with that fixed the two
    // are the same, and this test records the decision rather than leaving it implied.
    for reason in [
        ExitReason::Clean,
        ExitReason::Failed,
        ExitReason::HostLost,
        ExitReason::StoppedByOperator,
    ] {
        assert_eq!(
            RestartPolicy::Always.should_restart(reason, 0),
            RestartPolicy::UnlessStopped.should_restart(reason, 0),
            "{reason:?}"
        );
    }
}

#[test]
fn on_failure_ignores_a_clean_exit_but_not_a_crash() {
    let policy = RestartPolicy::OnFailure { max_retries: None };

    assert!(!policy.should_restart(ExitReason::Clean, 0));
    assert!(policy.should_restart(ExitReason::Failed, 0));
    assert!(policy.should_restart(ExitReason::HostLost, 0));
}

#[test]
fn a_bounded_on_failure_policy_stops_after_its_budget() {
    let policy = RestartPolicy::OnFailure {
        max_retries: Some(2),
    };

    assert!(policy.should_restart(ExitReason::Failed, 0));
    assert!(policy.should_restart(ExitReason::Failed, 1));
    assert!(
        !policy.should_restart(ExitReason::Failed, 2),
        "budget was exhausted"
    );
}

#[test]
fn policies_survive_a_round_trip_through_their_record_form() {
    for policy in [
        RestartPolicy::No,
        RestartPolicy::OnFailure { max_retries: None },
        RestartPolicy::OnFailure {
            max_retries: Some(5),
        },
        RestartPolicy::Always,
        RestartPolicy::UnlessStopped,
    ] {
        let text = policy.to_record();
        assert_eq!(
            RestartPolicy::parse_record(&text),
            Some(policy),
            "record form {text:?}"
        );
    }
}

// --- durable records ---

fn store() -> ChamberStore {
    let unique = format!(
        "dome-state-test-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    );
    let root = std::env::temp_dir().join(unique.replace(['(', ')', ' '], ""));
    let _ = std::fs::remove_dir_all(&root);
    ChamberStore::at(root)
}

#[test]
fn a_record_round_trips() {
    let store = store();
    let saved = record("web", RestartPolicy::Always, ExitReason::Failed, true);

    store.save(&saved).expect("save");
    let loaded = store.load(&saved.id).expect("load").expect("present");

    assert_eq!(loaded, saved);
    let _ = std::fs::remove_dir_all(store.root());
}

#[test]
fn saving_leaves_no_temporary_file_behind() {
    let store = store();
    let saved = record("web", RestartPolicy::Always, ExitReason::Clean, true);

    store.save(&saved).expect("save");
    store.save(&saved).expect("save again");

    let leftovers: Vec<_> = std::fs::read_dir(store.root())
        .expect("read dir")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".tmp"))
        .collect();

    assert!(
        leftovers.is_empty(),
        "temporary files left behind: {leftovers:?}"
    );
    let _ = std::fs::remove_dir_all(store.root());
}

#[test]
fn one_corrupt_record_does_not_hide_the_others() {
    let store = store();
    store
        .save(&record(
            "good",
            RestartPolicy::Always,
            ExitReason::Clean,
            true,
        ))
        .expect("save");

    std::fs::write(store.root().join("broken.chamber"), "policy=nonsense\n").expect("write");

    let (records, rejected) = store.load_all().expect("load all");

    assert_eq!(records.len(), 1);
    assert_eq!(records[0].id.as_str(), "good");
    assert_eq!(rejected.len(), 1, "the corrupt file should be reported");
    let _ = std::fs::remove_dir_all(store.root());
}

#[test]
fn reconcile_resumes_what_was_running_under_a_permissive_policy() {
    let store = store();
    store
        .save(&record(
            "always",
            RestartPolicy::Always,
            ExitReason::Clean,
            true,
        ))
        .expect("save");
    store
        .save(&record("never", RestartPolicy::No, ExitReason::Clean, true))
        .expect("save");

    let resume = store.reconcile().expect("reconcile");

    assert_eq!(resume.len(), 1);
    assert_eq!(resume[0].id.as_str(), "always");
    let _ = std::fs::remove_dir_all(store.root());
}

#[test]
fn reconcile_does_not_resume_something_an_operator_stopped() {
    let store = store();
    // Running when written, but the last recorded reason was a deliberate stop.
    store
        .save(&record(
            "stopped",
            RestartPolicy::Always,
            ExitReason::StoppedByOperator,
            false,
        ))
        .expect("save");

    let resume = store.reconcile().expect("reconcile");
    assert!(
        resume.is_empty(),
        "a deliberate stop must not be undone by a restart"
    );
    let _ = std::fs::remove_dir_all(store.root());
}

#[test]
fn a_hostile_id_cannot_escape_the_state_directory() {
    let store = store();
    for hostile in ["../escape", "a/b", "with space"] {
        let Ok(id) = ChamberId::new(hostile) else {
            continue;
        };
        let result = store.path_for(&id);

        if let Ok(path) = result {
            assert!(
                path.starts_with(store.root()),
                "{hostile:?} escaped to {}",
                path.display()
            );
        }
    }
}
