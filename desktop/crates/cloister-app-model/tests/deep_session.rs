//! The app's behaviour through its highest seam: the model talking to a real enforcement
//! service over IPC (recording platform, fake clock), exactly as the GPUI app does.

use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Duration, Utc};
use cloister_app_model::{BeginForm, Length, Missing, NoticeKind, SocketEnforcer, Today, Zone};
use cloister_core::Enforcement;
use cloister_enforcer::ipc;
use cloister_enforcer::service::shared_domains;
use cloister_enforcer::store::StateStore;
use cloister_enforcer::{Clock, Platform, Service};

#[derive(Clone, Default)]
struct Recorder(Arc<Mutex<Vec<Enforcement>>>);

impl Platform for Recorder {
    fn apply(&mut self, e: &Enforcement) -> io::Result<()> {
        self.0.lock().unwrap().push(e.clone());
        Ok(())
    }
}

#[derive(Clone)]
struct FakeClock(Arc<Mutex<DateTime<Utc>>>);

impl FakeClock {
    fn advance(&self, d: Duration) {
        *self.0.lock().unwrap() += d;
    }
    fn now(&self) -> DateTime<Utc> {
        *self.0.lock().unwrap()
    }
}

impl Clock for FakeClock {
    fn now(&self) -> DateTime<Utc> {
        *self.0.lock().unwrap()
    }
}

/// A running dry-run enforcement service.
struct Enforcer {
    _dir: tempfile::TempDir,
    socket: PathBuf,
    clock: FakeClock,
    platform: Recorder,
}

impl Enforcer {
    fn start(at: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let clock = FakeClock(Arc::new(Mutex::new(at.parse().unwrap())));
        let platform = Recorder::default();
        let service = Service::open(
            StateStore::new(dir.path().join("state.json")),
            platform.clone(),
            clock.clone(),
            shared_domains(),
        )
        .unwrap();
        // Unix socket paths must be short, so they live in /tmp rather than $TMPDIR.
        let name = dir
            .path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let socket = PathBuf::from(format!("/tmp/cla-{name}.sock"));
        ipc::serve(&socket, Arc::new(Mutex::new(service))).unwrap();
        Enforcer {
            _dir: dir,
            socket,
            clock,
            platform,
        }
    }

    /// The app's model, with times shown in UTC so expectations don't depend on the Mac.
    fn today(&self) -> Today<SocketEnforcer> {
        Today::new(SocketEnforcer::new(&self.socket)).in_zone(Zone::Utc)
    }

    fn blocking(&self) -> bool {
        let applied = self.platform.0.lock().unwrap();
        applied
            .last()
            .is_some_and(|e| e.blocked_domains.contains(&"youtube.com".to_string()))
    }
}

fn deep_form(intent: &str, minutes: i64) -> BeginForm {
    BeginForm {
        intent: intent.into(),
        length: Length::Preset(minutes),
        ..BeginForm::default()
    }
}

#[test]
fn beginning_a_deep_session_shows_it_running_and_enforces_it() {
    let enforcer = Enforcer::start("2026-01-05T09:30:00Z");
    let mut today = enforcer.today();
    let now = enforcer.clock.now();
    today.refresh(now);
    assert!(today.view(now).active.is_none());

    today
        .begin(&deep_form("Draft the methods section.", 90), now)
        .unwrap();

    let view = today.view(now);
    let active = view.active.expect("a Deep Session is running");
    assert!(active.deep);
    assert_eq!(active.intent.as_deref(), Some("Draft the methods section."));
    assert_eq!(
        active.ends_at,
        "2026-01-05T11:00:00Z".parse::<DateTime<Utc>>().unwrap()
    );
    assert_eq!(active.remaining, Duration::minutes(90));
    assert!(enforcer.blocking());
}

#[test]
fn when_a_deep_session_ends_a_depth_rating_is_asked_for_until_given() {
    let enforcer = Enforcer::start("2026-01-05T09:30:00Z");
    let mut today = enforcer.today();
    today
        .begin(&deep_form("Outline the talk.", 45), enforcer.clock.now())
        .unwrap();
    assert!(
        today.view(enforcer.clock.now()).rating.is_none(),
        "not while it runs"
    );

    enforcer.clock.advance(Duration::minutes(45));
    let now = enforcer.clock.now();
    today.refresh(now);
    let view = today.view(now);
    assert!(view.active.is_none());
    let prompt = view.rating.expect("asked for a Depth Rating");
    assert_eq!(prompt.intent.as_deref(), Some("Outline the talk."));

    // Starting another Block doesn't make the prompt go away: it stays until answered.
    let plain = BeginForm {
        deep: false,
        ..BeginForm::default()
    };
    today.begin(&plain, now).unwrap();
    assert_eq!(today.view(now).rating.as_ref(), Some(&prompt));

    today.rate(&prompt.block_id, 4, now).unwrap();
    assert!(today.view(now).rating.is_none());
}

#[test]
fn a_plain_block_has_no_intent_and_skips_the_depth_rating() {
    let enforcer = Enforcer::start("2026-01-05T09:30:00Z");
    let mut today = enforcer.today();
    let mut form = deep_form("ignored when plain", 60);
    form.deep = false;
    today.begin(&form, enforcer.clock.now()).unwrap();
    let active = today.view(enforcer.clock.now()).active.unwrap();
    assert!(!active.deep);
    assert_eq!(active.intent, None);

    enforcer.clock.advance(Duration::minutes(60));
    today.refresh(enforcer.clock.now());
    assert!(today.view(enforcer.clock.now()).rating.is_none());
}

#[test]
fn a_deep_session_needs_an_intent_and_a_length() {
    let mut form = BeginForm::default();
    assert_eq!(form.missing(), Some(Missing::Intent));
    form.intent = "   ".into();
    assert_eq!(form.missing(), Some(Missing::Intent));
    form.deep = false;
    assert_eq!(form.missing(), None, "a plain Block needs no Intent");
    form.deep = true;
    form.intent = "Fix the parser.".into();
    form.length = Length::Custom(String::new());
    assert_eq!(form.missing(), Some(Missing::Length));
    form.length = Length::Custom("75".into());
    assert_eq!(form.missing(), None);
    assert_eq!(form.minutes(), Some(75));
}

#[test]
fn custom_lengths_over_24_hours_cant_be_entered() {
    assert!(BeginForm::custom_input_allowed(""));
    assert!(BeginForm::custom_input_allowed("1440"));
    assert!(!BeginForm::custom_input_allowed("1441"));
    assert!(!BeginForm::custom_input_allowed("12a"));
    assert!(!BeginForm::custom_input_allowed("-5"));
    assert!(!BeginForm::custom_input_allowed("99999999999999999999999"));
}

#[test]
fn blocks_over_4_hours_need_the_cant_be_undone_confirmation() {
    let enforcer = Enforcer::start("2026-01-05T09:30:00Z");
    let mut today = enforcer.today();
    let now = enforcer.clock.now();
    let mut form = deep_form("Write the grant.", 0);
    form.length = Length::Custom("300".into());
    assert_eq!(form.missing(), Some(Missing::LongConfirmation));
    assert!(today.begin(&form, now).is_err());
    assert!(today.view(now).active.is_none());

    form.long_confirmed = true;
    assert_eq!(form.missing(), None);
    today.begin(&form, now).unwrap();
    assert_eq!(
        today.view(now).active.unwrap().remaining,
        Duration::minutes(300)
    );

    // Back to a short length: the confirmation is no longer asked for.
    form.long_confirmed = false;
    form.length = Length::Preset(60);
    assert_eq!(form.missing(), None);
}

#[test]
fn the_summary_shows_start_end_and_deep_hours_after() {
    // Sunday: last ISO week.
    let enforcer = Enforcer::start("2026-01-04T10:00:00Z");
    let mut today = enforcer.today();
    today
        .begin(&deep_form("Last week's work.", 60), enforcer.clock.now())
        .unwrap();

    // Monday 09:30.
    enforcer.clock.advance(Duration::minutes(23 * 60 + 30));
    let now = enforcer.clock.now();
    today
        .begin(&deep_form("This week's work.", 45), now)
        .unwrap();
    let plain = BeginForm {
        deep: false,
        ..BeginForm::default()
    };
    today.begin(&plain, now).unwrap();
    assert_eq!(today.view(now).deep_hours_this_week, Duration::minutes(45));

    let summary = today.summary(&deep_form("Next.", 90), now).unwrap();
    assert_eq!(
        summary.starts_at,
        "2026-01-05T09:30:00Z".parse::<DateTime<Utc>>().unwrap()
    );
    assert_eq!(
        summary.ends_at,
        "2026-01-05T11:00:00Z".parse::<DateTime<Utc>>().unwrap()
    );
    assert_eq!(summary.deep_hours_after, Duration::minutes(135));
    assert_eq!(
        cloister_app_model::format::clock(summary.ends_at, &today.zone()),
        "11:00"
    );
    assert_eq!(
        cloister_app_model::format::hours(summary.deep_hours_after),
        "2h 15m"
    );

    let mut chores = deep_form("", 60);
    chores.deep = false;
    let summary = today.summary(&chores, now).unwrap();
    assert_eq!(
        summary.deep_hours_after,
        Duration::minutes(45),
        "a plain Block adds no Deep Hours"
    );
}

#[test]
fn reopening_the_app_shows_the_running_deep_session() {
    let enforcer = Enforcer::start("2026-01-05T09:30:00Z");
    let mut first = enforcer.today();
    first
        .begin(&deep_form("Survive a restart.", 60), enforcer.clock.now())
        .unwrap();
    drop(first);

    enforcer.clock.advance(Duration::minutes(10));
    let mut reopened = enforcer.today();
    reopened.refresh(enforcer.clock.now());
    let active = reopened.view(enforcer.clock.now()).active.unwrap();
    assert_eq!(active.intent.as_deref(), Some("Survive a restart."));
    assert_eq!(active.remaining, Duration::minutes(50));
}

#[test]
fn the_countdown_follows_server_time_not_the_local_clock() {
    let enforcer = Enforcer::start("2026-01-05T09:30:00Z");
    let mut today = enforcer.today();
    // This Device's clock runs 3 hours fast.
    let local = enforcer.clock.now() + Duration::hours(3);
    today
        .begin(&deep_form("Ignore the wall clock.", 60), local)
        .unwrap();
    assert_eq!(
        today.view(local).active.unwrap().remaining,
        Duration::minutes(60)
    );
    let later = local + Duration::seconds(61);
    assert_eq!(
        today.view(later).active.unwrap().remaining,
        Duration::minutes(58) + Duration::seconds(59)
    );
    assert_eq!(
        cloister_app_model::format::countdown(Duration::minutes(58) + Duration::seconds(59)),
        "58:59"
    );
    assert_eq!(
        cloister_app_model::format::countdown(Duration::minutes(90)),
        "1:30:00"
    );
}

#[test]
fn refusals_come_back_as_calm_notices_naming_the_time() {
    let enforcer = Enforcer::start("2026-01-05T09:30:00Z");
    let mut today = enforcer.today();
    let now = enforcer.clock.now();
    today.begin(&deep_form("Hold the line.", 90), now).unwrap();
    let block_id = today.view(now).active.unwrap().block_id;

    let notice = today.rate(&block_id, 5, now).unwrap_err();
    assert_eq!(notice.text, "Ends at 11:00. Rate it then.");
    assert_eq!(notice.kind, NoticeKind::WhatIsSo);
    for word in ["sorry", "oops", "error", "not allowed", "!"] {
        assert!(!notice.text.to_lowercase().contains(word));
    }
}

#[test]
fn without_the_enforcement_service_nothing_starts_and_it_says_so() {
    let mut today = Today::new(SocketEnforcer::new("/tmp/cloister-nobody-here.sock"));
    let now: DateTime<Utc> = "2026-01-05T09:30:00Z".parse().unwrap();
    today.refresh(now);
    assert!(!today.view(now).reachable);
    let notice = today.begin(&deep_form("Anything.", 45), now).unwrap_err();
    assert_eq!(notice.kind, NoticeKind::WhatIsSo);
    assert!(
        notice.text.contains("enforcement service"),
        "{}",
        notice.text
    );
}

// Evening Lockdown and Rest Days ------------------------------------------------------------

fn settle_settings(enforcer: &Enforcer) {
    let answer = ipc::send(
        &enforcer.socket,
        &ipc::Request::Sync {
            blocks: vec![],
            blocklists: None,
            access_token: None,
            planning: Some(ipc::Planning {
                settings: Some(cloister_core::Settings {
                    workdays: vec![1, 2, 3, 4, 5],
                    workday_start: "09:00".into(),
                    shutdown_time: "17:00".into(),
                }),
                ..ipc::Planning::default()
            }),
        },
    )
    .unwrap();
    assert!(matches!(answer, ipc::Response::Accepted { .. }));
}

#[test]
fn at_shutdown_time_the_app_says_the_workday_has_ended_and_when_the_next_starts() {
    let enforcer = Enforcer::start("2026-01-05T16:59:00Z");
    settle_settings(&enforcer);
    let mut today = enforcer.today();
    let now = enforcer.clock.now();
    today.refresh(now);
    assert!(today.view(now).ended.is_none());
    assert!(!today.view(now).night());

    enforcer.clock.advance(Duration::minutes(1));
    let now = enforcer.clock.now();
    today.refresh(now);
    let view = today.view(now);
    let ended = view.ended.clone().expect("Evening Lockdown");
    assert!(view.night());
    assert_eq!(ended.headline, "The workday has ended.");
    assert!(!ended.rest_day);
    assert_eq!(
        ended.next_workday_start,
        Some("2026-01-06T09:00:00Z".parse().unwrap())
    );
    assert_eq!(ended.next_start_text, "Workday Start Tue 6 Jan, 09:00.");

    // No Deep Session now: a calm notice naming when work starts again.
    let refusal = today
        .begin(&deep_form("One more thing", 60), now)
        .unwrap_err();
    assert_eq!(refusal.kind, NoticeKind::WhatIsSo);
    assert_eq!(
        refusal.text,
        "Evening Lockdown. Deep Sessions start again at 09:00."
    );
}

#[test]
fn a_rest_day_is_night_all_day_and_refuses_deep_sessions() {
    let enforcer = Enforcer::start("2026-01-10T12:00:00Z");
    settle_settings(&enforcer);
    let mut today = enforcer.today();
    let now = enforcer.clock.now();
    today.refresh(now);
    let view = today.view(now);
    assert!(view.night());
    let ended = view.ended.expect("Rest Day");
    assert!(ended.rest_day);
    assert_eq!(ended.next_start_text, "Workday Start Mon 12 Jan, 09:00.");
    let refusal = today
        .begin(&deep_form("Weekend push", 60), now)
        .unwrap_err();
    assert_eq!(
        refusal.text,
        "A Rest Day. Deep Sessions start again Mon 12 Jan, 09:00."
    );
    assert!(
        enforcer.blocking(),
        "the Distraction List is blocked all day"
    );
}
