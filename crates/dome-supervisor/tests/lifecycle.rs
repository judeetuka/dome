//! Lifecycle and health rules.
//!
//! The health tracker is deliberately clock-free so its rules can be pinned exactly as the
//! Docker documentation describes them, without any test ever sleeping.

use std::time::Duration;

use dome_supervisor::lifecycle::{HealthPolicy, HealthState, HealthTracker, StopPolicy};

fn policy() -> HealthPolicy {
    HealthPolicy {
        interval: Duration::from_secs(30),
        timeout: Duration::from_secs(30),
        retries: 3,
        start_period: Duration::from_secs(0),
    }
}

#[test]
fn health_starts_unknown_to_the_tracker_until_the_first_probe() {
    let tracker = HealthTracker::new(policy());
    assert_eq!(tracker.state(), HealthState::Starting);
}

#[test]
fn a_success_makes_it_healthy() {
    let mut tracker = HealthTracker::new(policy());
    assert_eq!(
        tracker.record(true, Duration::from_secs(1)),
        HealthState::Healthy
    );
}

#[test]
fn consecutive_failures_reach_unhealthy_at_the_retry_limit() {
    let mut tracker = HealthTracker::new(policy());
    let after = Duration::from_secs(1);

    assert_eq!(tracker.record(false, after), HealthState::Starting);
    assert_eq!(tracker.record(false, after), HealthState::Starting);
    // Third consecutive failure: the limit from HealthPolicy::default.
    assert_eq!(tracker.record(false, after), HealthState::Unhealthy);
}

#[test]
fn a_success_resets_the_failure_count() {
    let mut tracker = HealthTracker::new(policy());
    let after = Duration::from_secs(1);

    tracker.record(false, after);
    tracker.record(false, after);
    tracker.record(true, after);

    // The counter restarted, so two more failures are not enough.
    assert_eq!(tracker.record(false, after), HealthState::Healthy);
    assert_eq!(tracker.record(false, after), HealthState::Healthy);
    assert_eq!(tracker.record(false, after), HealthState::Unhealthy);
}

#[test]
fn failures_during_the_start_period_are_not_counted() {
    let mut tracker = HealthTracker::new(HealthPolicy {
        start_period: Duration::from_secs(60),
        ..policy()
    });

    // Ten failures well inside the start period still leave it starting, never unhealthy.
    for _ in 0..10 {
        assert_eq!(
            tracker.record(false, Duration::from_secs(5)),
            HealthState::Starting
        );
    }
}

#[test]
fn a_success_inside_the_start_period_ends_it() {
    let mut tracker = HealthTracker::new(HealthPolicy {
        start_period: Duration::from_secs(60),
        ..policy()
    });

    tracker.record(true, Duration::from_secs(10));
    assert_eq!(tracker.state(), HealthState::Healthy);

    // Now that it is healthy, failures are counted even though the clock is still inside the
    // start period - which is what the Docker documentation specifies.
    tracker.record(false, Duration::from_secs(11));
    tracker.record(false, Duration::from_secs(12));
    assert_eq!(
        tracker.record(false, Duration::from_secs(13)),
        HealthState::Unhealthy
    );
}

#[test]
fn the_default_stop_policy_matches_dockers_behaviour() {
    let policy = StopPolicy::default();
    assert_eq!(
        policy.signal,
        libc::SIGTERM,
        "docker stop defaults to SIGTERM"
    );
    assert_eq!(
        policy.grace,
        Some(Duration::from_secs(10)),
        "docker stop waits ten seconds before forcing"
    );
}
