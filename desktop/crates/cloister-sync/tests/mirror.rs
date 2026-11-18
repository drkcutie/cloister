//! The local SQLite mirror: the enforcement source on a Device, online or offline.

use chrono::{DateTime, Utc};
use cloister_core::{enforcement, Block};
use cloister_sync::Mirror;

fn t(s: &str) -> DateTime<Utc> {
    s.parse().unwrap()
}

fn block(id: &str, starts: &str, ends: &str) -> Block {
    Block {
        id: id.into(),
        starts_at: t(starts),
        ends_at: t(ends),
        blocklists: vec![
            "forums".into(),
            "news".into(),
            "shopping".into(),
            "social".into(),
            "video".into(),
        ],
        deep: true,
        intent: None,
        depth_rating: None,
    }
}

#[test]
fn blocks_from_the_backend_survive_a_restart_and_keep_being_enforced() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mirror.sqlite");
    {
        let mut mirror = Mirror::open(&path).unwrap();
        mirror
            .merge_blocks(&[block("b1", "2026-01-05T09:00:00Z", "2026-01-05T10:00:00Z")])
            .unwrap();
    }
    let mirror = Mirror::open(&path).unwrap();
    let state = mirror.state().unwrap();
    assert_eq!(state.blocks.len(), 1);
    let now = enforcement(&state, t("2026-01-05T09:30:00Z"));
    assert!(now.blocked_domains.contains(&"youtube.com".to_string()));
}

#[test]
fn merging_never_shortens_or_forgets_a_block() {
    let dir = tempfile::tempdir().unwrap();
    let mut mirror = Mirror::open(dir.path().join("m.sqlite")).unwrap();
    mirror
        .merge_blocks(&[
            block("b1", "2026-01-05T09:00:00Z", "2026-01-05T11:00:00Z"),
            block("b2", "2026-01-05T09:05:00Z", "2026-01-05T09:35:00Z"),
        ])
        .unwrap();
    // A tampered or stale answer: b1 shorter, b2 missing.
    let changed = mirror
        .merge_blocks(&[block("b1", "2026-01-05T09:00:00Z", "2026-01-05T09:10:00Z")])
        .unwrap();
    assert!(!changed);
    let state = mirror.state().unwrap();
    let ids: Vec<_> = state
        .blocks
        .iter()
        .map(|b| (b.id.as_str(), b.ends_at))
        .collect();
    assert_eq!(
        ids,
        vec![
            ("b1", t("2026-01-05T11:00:00Z")),
            ("b2", t("2026-01-05T09:35:00Z"))
        ]
    );
    // Extensions do arrive.
    let changed = mirror
        .merge_blocks(&[block("b1", "2026-01-05T09:00:00Z", "2026-01-05T12:00:00Z")])
        .unwrap();
    assert!(changed);
    assert_eq!(
        mirror.state().unwrap().blocks[0].ends_at,
        t("2026-01-05T12:00:00Z")
    );
}

#[test]
fn the_device_id_is_generated_once() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("m.sqlite");
    let first = Mirror::open(&path).unwrap().device_id().unwrap();
    let again = Mirror::open(&path).unwrap().device_id().unwrap();
    assert_eq!(first, again);
    assert_eq!(first.len(), 36, "a UUID: {first}");
}

fn day_plan(slot_id: &str, deep: bool) -> cloister_core::DayPlan {
    let slot = cloister_core::Slot {
        id: slot_id.into(),
        starts_at: t("2026-01-05T09:00:00Z"),
        ends_at: t("2026-01-05T10:00:00Z"),
        label: "Draft chapter 3".into(),
        deep,
        depth_rating: None,
    };
    cloister_core::DayPlan {
        date: "2026-01-05".parse().unwrap(),
        starts_at: t("2026-01-05T08:00:00Z"),
        ends_at: t("2026-01-05T17:00:00Z"),
        planned_deep_minutes: if deep { 60 } else { 0 },
        slots: vec![slot],
    }
}

#[test]
fn day_plans_from_the_backend_survive_a_restart_and_deep_slots_are_enforced_offline() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mirror.sqlite");
    {
        let mut mirror = Mirror::open(&path).unwrap();
        assert!(mirror
            .replace_day_plans(&[day_plan("s1", true)], &[])
            .unwrap());
        assert!(!mirror
            .replace_day_plans(&[day_plan("s1", true)], &[])
            .unwrap());
    }
    let mirror = Mirror::open(&path).unwrap();
    let state = mirror.state().unwrap();
    assert_eq!(state.day_plans, vec![day_plan("s1", true)]);
    let during = enforcement(&state, t("2026-01-05T09:30:00Z"));
    assert!(during.blocked_domains.contains(&"youtube.com".to_string()));
    assert!(enforcement(&state, t("2026-01-05T10:00:00Z"))
        .blocked_domains
        .is_empty());
}

#[test]
fn the_backend_is_the_authority_on_day_plans() {
    let dir = tempfile::tempdir().unwrap();
    let mut mirror = Mirror::open(dir.path().join("mirror.sqlite")).unwrap();
    mirror
        .replace_day_plans(&[day_plan("s1", true)], &[])
        .unwrap();
    // The backend's ratchet already decided; a moved Slot replaces the old one.
    assert!(mirror
        .replace_day_plans(&[day_plan("s2", true)], &[])
        .unwrap());
    let state = mirror.state().unwrap();
    assert_eq!(state.day_plans, vec![day_plan("s2", true)]);
}
