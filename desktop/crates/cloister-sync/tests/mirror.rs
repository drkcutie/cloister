//! The local SQLite mirror: the enforcement source on a Device, online or offline.

use chrono::{DateTime, Utc};
use cloister_core::{
    enforcement, Block, BlocklistEdits, BlocklistEntry, BlocklistRemoval, EntryKind,
};
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
        extra_domains: vec![],
        extra_apps: vec![],
        quit_social: false,
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

#[test]
fn blocklist_edits_from_the_backend_never_loosen_before_tomorrow_here() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("m.sqlite");
    let removal = |effective_at: &str| BlocklistRemoval {
        blocklist: "forums".into(),
        kind: EntryKind::Domain,
        entry: "reddit.com".into(),
        effective_at: t(effective_at),
    };
    let added = BlocklistEntry {
        blocklist: "work".into(),
        kind: EntryKind::App,
        entry: "Figma".into(),
    };
    {
        let mut mirror = Mirror::open(&path).unwrap();
        let mut quit = block("q1", "2026-01-05T09:00:00Z", "2026-02-04T09:00:00Z");
        quit.blocklists = vec!["social".into()];
        quit.quit_social = true;
        quit.extra_domains = vec!["example.com".into()];
        mirror.merge_blocks(&[quit]).unwrap();
        let changed = mirror
            .merge_blocklist_edits(
                "Europe/Paris",
                &BlocklistEdits {
                    entries: vec![added.clone()],
                    // Claims to be in force already: here it waits for the next midnight.
                    removals: vec![removal("2026-01-05T00:00:00Z")],
                    opt_outs: vec![],
                },
                t("2026-01-05T09:00:00Z"),
            )
            .unwrap();
        assert!(changed);
    }
    let mut mirror = Mirror::open(&path).unwrap();
    let state = mirror.state().unwrap();
    assert_eq!(state.time_zone, "Europe/Paris");
    assert!(state.blocks[0].quit_social);
    assert_eq!(state.blocks[0].extra_domains, ["example.com"]);
    assert_eq!(state.blocklist_entries, [added.clone()]);
    assert_eq!(state.blocklist_removals, [removal("2026-01-05T23:00:00Z")]);

    // The backend dropped the removal (the entry was added back) and lost the addition:
    // the removal goes, the addition stays.
    mirror
        .merge_blocklist_edits(
            "Europe/Paris",
            &BlocklistEdits::default(),
            t("2026-01-05T10:00:00Z"),
        )
        .unwrap();
    let state = mirror.state().unwrap();
    assert!(state.blocklist_removals.is_empty());
    assert_eq!(state.blocklist_entries, [added]);
}
