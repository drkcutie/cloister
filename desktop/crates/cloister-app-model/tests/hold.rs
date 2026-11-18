//! Hold-to-commit: two seconds of holding (pointer or key) commits; letting go early doesn't.

use std::time::{Duration, Instant};

use cloister_app_model::Hold;

#[test]
fn holding_for_two_seconds_commits_once() {
    let t0 = Instant::now();
    let mut hold = Hold::default();
    assert_eq!(hold.progress(t0), 0.0);

    hold.press(t0);
    assert!(hold.is_holding());
    assert_eq!(hold.poll(t0 + Duration::from_millis(1000)), None);
    assert!((hold.progress(t0 + Duration::from_millis(1000)) - 0.5).abs() < 1e-6);
    assert_eq!(hold.poll(t0 + Duration::from_millis(2000)), Some(()));
    assert!(!hold.is_holding(), "a commit ends the hold");
    assert_eq!(
        hold.poll(t0 + Duration::from_millis(2500)),
        None,
        "commits once"
    );
}

#[test]
fn letting_go_early_commits_nothing_and_starts_over() {
    let t0 = Instant::now();
    let mut hold = Hold::default();
    hold.press(t0);
    hold.release();
    assert_eq!(hold.poll(t0 + Duration::from_secs(3)), None);
    assert_eq!(hold.progress(t0 + Duration::from_secs(3)), 0.0);
}

#[test]
fn key_repeat_does_not_restart_the_hold() {
    let t0 = Instant::now();
    let mut hold = Hold::default();
    hold.press(t0);
    hold.press(t0 + Duration::from_millis(1500)); // repeated key-down while held
    assert_eq!(hold.poll(t0 + Duration::from_millis(2000)), Some(()));
}
