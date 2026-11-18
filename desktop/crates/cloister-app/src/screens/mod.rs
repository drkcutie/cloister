//! The main window. One view, [`MainView`], picks the screen: Today (Begin, with the Depth
//! Rating prompt when one is owed, or the running Deep Session), Blocklists or Devices,
//! chosen in the sidebar.
//!
//! To add a screen: add a module here with a `render_*` function taking `&mut MainView`
//! (or a separate `Render` entity if it owns inputs), and choose it in `MainView::render`.

mod begin;
mod blocklists;
pub(crate) mod day_plan;
mod devices;
mod ended;
mod rating;
mod session;
mod settings;
mod sidebar;
pub(crate) mod weekly_review;

use std::time::{Duration, Instant};

use chrono::Utc;
use cloister_app_model::{BeginForm, Hold, Length, Notice, SettingsForm};
use cloister_core::EntryKind;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;

use crate::state::{Cloister, CloisterHandle};
use crate::theme::{PaletteExt, SANS};
use crate::ui;

/// The screen picked in the sidebar (a running Deep Session takes over Today).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Today,
    DayPlan,
    Blocklists,
    WeeklyReview,
    Devices,
    Settings,
}

pub struct MainView {
    cloister: Entity<Cloister>,
    screen: Screen,
    /// The site or app being added on the Blocklists screen, and to which Blocklist.
    entry: Entity<InputState>,
    entry_list: String,
    /// A site or app for this Block only, on the Begin screen.
    extra: Entity<InputState>,
    /// The Quit-Social Block's "can't be undone" confirmation is showing.
    confirm_quit_social: bool,
    day_plan: Option<Entity<day_plan::DayPlanView>>,
    /// The Settings form while that screen is open (reopened each time it is shown).
    settings: Option<SettingsForm>,
    settings_time_zone: Entity<InputState>,
    settings_notice: Option<Notice>,
    form: BeginForm,
    intent: Entity<InputState>,
    custom: Entity<InputState>,
    /// The last text the custom length field held that was allowed.
    custom_text: String,
    hold: Hold,
    hold_focus: FocusHandle,
    _hold_task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl MainView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let cloister = cx.global::<CloisterHandle>().0.clone();
        let intent = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Draft the methods section, start to finish.")
        });
        let custom = cx.new(|cx| InputState::new(window, cx).placeholder("75"));
        let entry = cx.new(|cx| InputState::new(window, cx).placeholder("example.com or Discord"));
        let extra = cx.new(|cx| InputState::new(window, cx).placeholder("example.com or Steam"));
        let settings_time_zone =
            cx.new(|cx| InputState::new(window, cx).placeholder("Europe/Paris"));
        let subscriptions = vec![
            cx.observe(&cloister, |_, _, cx| cx.notify()),
            cx.subscribe_in(&settings_time_zone, window, |this, state, event, _, cx| {
                if let InputEvent::Change = event {
                    let tz = state.read(cx).value().to_string();
                    if let Some(f) = this.settings.as_mut() {
                        f.set_time_zone(&tz);
                    }
                    cx.notify();
                }
            }),
            cx.subscribe_in(&intent, window, |this, state, event, _, cx| {
                if let InputEvent::Change = event {
                    this.form.intent = state.read(cx).value().to_string();
                    cx.notify();
                }
            }),
            cx.subscribe_in(&custom, window, |this, state, event, window, cx| {
                if let InputEvent::Change = event {
                    let text = state.read(cx).value().to_string();
                    if BeginForm::custom_input_allowed(&text) {
                        this.custom_text = text.clone();
                        if !text.is_empty() || matches!(this.form.length, Length::Custom(_)) {
                            this.form.length = Length::Custom(text);
                        }
                    } else {
                        // Over 24 hours (or not a number) can't be entered at all.
                        let previous = this.custom_text.clone();
                        state.update(cx, |s, cx| s.set_value(previous, window, cx));
                    }
                    cx.notify();
                }
            }),
        ];
        intent.update(cx, |s, cx| s.focus(window, cx));
        MainView {
            cloister,
            // Opening on another screen helps screenshots and manual checks.
            screen: match std::env::var("CLOISTER_SCREEN").as_deref() {
                Ok("blocklists") => Screen::Blocklists,
                Ok("devices") => Screen::Devices,
                Ok("day-plan") => Screen::DayPlan,
                Ok("weekly-review") => Screen::WeeklyReview,
                Ok("settings") => Screen::Settings,
                _ => Screen::Today,
            },
            entry,
            entry_list: "social".into(),
            extra,
            confirm_quit_social: false,
            day_plan: None,
            settings: None,
            settings_time_zone,
            settings_notice: None,
            form: BeginForm::default(),
            intent,
            custom,
            custom_text: String::new(),
            hold: Hold::default(),
            hold_focus: cx.focus_handle(),
            _hold_task: None,
            _subscriptions: subscriptions,
        }
    }

    fn choose_preset(&mut self, minutes: i64, window: &mut Window, cx: &mut Context<Self>) {
        self.form.length = Length::Preset(minutes);
        self.custom_text.clear();
        self.custom.update(cx, |s, cx| s.set_value("", window, cx));
        cx.notify();
    }

    pub(crate) fn show(&mut self, screen: Screen, window: &mut Window, cx: &mut Context<Self>) {
        self.screen = screen;
        if screen == Screen::DayPlan && self.day_plan.is_none() {
            self.day_plan = Some(cx.new(|cx| day_plan::DayPlanView::new(window, cx)));
        }
        if screen == Screen::Settings {
            self.settings = None;
            self.settings_notice = None;
            if let Some(device) = self.cloister.read(cx).account() {
                let form = SettingsForm::open(device);
                let tz = form.time_zone().to_string();
                self.settings_time_zone
                    .update(cx, |s, cx| s.set_value(tz, window, cx));
                self.settings = Some(form);
            }
        }
        self.confirm_quit_social = false;
        self.cloister.update(cx, |c, cx| c.dismiss_notice(cx));
    }

    fn save_settings(&mut self, cx: &mut Context<Self>) {
        let Some(device) = self.cloister.read(cx).account().cloned() else {
            return;
        };
        if let Some(form) = self.settings.as_mut() {
            self.settings_notice = Some(form.save(&device));
        }
        cx.notify();
    }

    /// The Day Plan screen on the next Workday: today before Workday Start, else tomorrow.
    fn show_next_day_plan(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show(Screen::DayPlan, window, cx);
        let next = self
            .cloister
            .read(cx)
            .view()
            .ended
            .and_then(|e| e.next_workday_start);
        let today = chrono::Local::now().date_naive();
        let day = match next {
            Some(t) if t.with_timezone(&chrono::Local).date_naive() == today => {
                day_plan::Day::Today
            }
            _ => day_plan::Day::Tomorrow,
        };
        if let Some(plan) = &self.day_plan {
            plan.update(cx, |plan, cx| plan.show(day, cx));
        }
    }

    /// Add what is typed in the Blocklists screen's field to the chosen Blocklist.
    fn add_entry(&mut self, kind: EntryKind, window: &mut Window, cx: &mut Context<Self>) {
        let input = self.entry.read(cx).value().to_string();
        if input.trim().is_empty() {
            return;
        }
        let list = self.entry_list.clone();
        let added = self
            .cloister
            .update(cx, |c, cx| c.add_entry(&list, kind, &input, cx));
        if added {
            self.entry.update(cx, |s, cx| s.set_value("", window, cx));
        }
    }

    /// Add what is typed in the Begin screen's extras field to this Block only.
    fn add_extra(&mut self, kind: EntryKind, window: &mut Window, cx: &mut Context<Self>) {
        let input = self.extra.read(cx).value().to_string();
        if self.form.add_extra(kind, &input) {
            self.extra.update(cx, |s, cx| s.set_value("", window, cx));
        }
        cx.notify();
    }

    fn press_hold(&mut self, cx: &mut Context<Self>) {
        if self.form.missing().is_some() || self.hold.is_holding() {
            return;
        }
        self.hold.press(Instant::now());
        self._hold_task = Some(cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(16))
                .await;
            let keep_going = this.update(cx, |this, cx| {
                let committed = this.hold.poll(Instant::now()).is_some();
                if committed {
                    this.commit(cx);
                }
                cx.notify();
                this.hold.is_holding()
            });
            if !matches!(keep_going, Ok(true)) {
                break;
            }
        }));
        cx.notify();
    }

    fn release_hold(&mut self, cx: &mut Context<Self>) {
        self.hold.release();
        self._hold_task = None;
        cx.notify();
    }

    fn commit(&mut self, cx: &mut Context<Self>) {
        let form = self.form.clone();
        let started = self.cloister.update(cx, |c, cx| c.begin(&form, cx));
        if started {
            self.form.long_confirmed = false;
            self.form.extra_domains.clear();
            self.form.extra_apps.clear();
        }
    }
}

impl Render for MainView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = cx.palette();
        let view = self.cloister.read(cx).view();
        let root = div()
            .size_full()
            .bg(p.ground)
            .text_color(p.ink)
            .font_family(SANS);
        match (self.screen, view.active.clone()) {
            // The workday has ended: Night, and nothing to begin. A Block still running (a plain
            // one, or a Deep Session that began before Shutdown Time) stays the hero until it ends.
            (Screen::Today, None) if view.ended.is_some() && view.rating.is_none() => {
                let ended = view.ended.clone().expect("checked");
                root.child(ui::fade_in("ended", ended::render(self, &view, &ended, cx)))
            }
            (Screen::Today, Some(active)) => root.child(ui::fade_in(
                SharedString::from(format!("session-{}", active.block_id)),
                session::render(self, &active, &view, window, cx),
            )),
            (Screen::Today, None) => {
                root.child(ui::fade_in("begin", begin::render(self, &view, window, cx)))
            }
            (Screen::Blocklists, _) => {
                let content = blocklists::render(self, window, cx);
                root.child(ui::fade_in(
                    "blocklists",
                    sidebar::with_sidebar(self, &view, content, cx),
                ))
            }
            (Screen::WeeklyReview, _) => {
                let content = weekly_review::render(self, cx);
                root.child(ui::fade_in(
                    "weekly-review",
                    sidebar::with_sidebar(self, &view, content, cx),
                ))
            }
            (Screen::Settings, _) => {
                let content = settings::render(self, cx);
                root.child(ui::fade_in(
                    "settings",
                    sidebar::with_sidebar(self, &view, content, cx),
                ))
            }
            (Screen::Devices, _) => {
                let content = devices::render(self, cx);
                root.child(ui::fade_in(
                    "devices",
                    sidebar::with_sidebar(self, &view, content, cx),
                ))
            }
            (Screen::DayPlan, _) => {
                let plan = self
                    .day_plan
                    .get_or_insert_with(|| cx.new(|cx| day_plan::DayPlanView::new(window, cx)))
                    .clone();
                let side = sidebar::render(self, &view, cx);
                root.child(ui::fade_in(
                    "day-plan",
                    div()
                        .size_full()
                        .flex()
                        .child(side)
                        .child(div().flex_1().min_w_0().h_full().child(plan)),
                ))
            }
        }
    }
}

/// Open the main window, or bring it forward if it is already open.
pub fn open_main_window(cx: &mut App) {
    if let Some(window) = cx.windows().into_iter().next() {
        let _ = window.update(cx, |_, window, _| window.activate_window());
        cx.activate(true);
        return;
    }
    let bounds = Bounds::centered(None, size(px(1280.), px(800.)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        window_min_size: Some(size(px(900.), px(640.))),
        titlebar: Some(TitlebarOptions {
            title: Some("Cloister".into()),
            ..Default::default()
        }),
        ..Default::default()
    };
    if let Err(e) = gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| {
            let mut view = MainView::new(window, cx);
            // Development aid: `CLOISTER_OPEN=day-plan|weekly-review|settings` opens on that screen.
            let open = match std::env::var("CLOISTER_OPEN").as_deref() {
                Ok("day-plan") => Some(Screen::DayPlan),
                Ok("weekly-review") => Some(Screen::WeeklyReview),
                Ok("settings") => Some(Screen::Settings),
                _ => None,
            };
            if let Some(screen) = open {
                view.show(screen, window, cx);
            }
            view
        })
    }) {
        eprintln!("cloister-app: opening the window failed: {e}");
    }
    cx.activate(true);
}

/// Local time now, for screens that need it.
pub(crate) fn now() -> chrono::DateTime<Utc> {
    Utc::now()
}
