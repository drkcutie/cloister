//! The Shutdown Ritual window (`cloister-app --ritual`), started by the ritual launch agent
//! while cloisterd says the ritual is due.
//!
//! It covers every display in the Night theme, under the kiosk lock ([`kiosk`]). The main
//! display walks the five steps (six with the Weekly Review on the last Workday of the week,
//! brass-numbered), with the Shutdown Delay at hand while one is left; the others say where the ritual is. Once a
//! second it asks cloisterd whether the ritual is still due, and quits when it isn't
//! (completed here or on another Device, or lapsed at Workday Start). Completing hands the
//! account's records to cloisterd at once, so the window goes within a second.

mod kiosk;

use std::time::{Duration, Instant};

use cloister_app_model::format::hours;
use cloister_app_model::{
    scoreboard, shutdown_delay, use_shutdown_delay, Enforcer, Hold, Notice, NoticeKind,
    PlanBackend, RitualFlow, RitualStep, SocketEnforcer, Zone,
};
use cloister_core::RitualScreen;
use cloister_enforcer::ipc::{Request, Response};
use cloister_sync::{Device, SyncHandle, SyncOptions};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::screens::day_plan::{Day, DayPlanView, Plans};
use crate::theme::{depth, radius, space, Palette, PaletteExt, Type, SANS};
use crate::ui;

pub use kiosk::Kiosk;

/// How often the window asks cloisterd whether the ritual is still due, and re-asserts the lock.
const POLL: Duration = Duration::from_secs(1);

/// Open the ritual on every display and keep it there.
pub fn run(cx: &mut App) {
    crate::theme::use_palette(cx, Palette::night());
    let kiosk = Kiosk::from_env();
    let primary = cx.primary_display().map(|d| d.id());
    let mut displays = cx.displays();
    // The main display first, so it takes focus.
    displays.sort_by_key(|d| Some(d.id()) != primary);
    for display in displays {
        let main = Some(display.id()) == primary || primary.is_none();
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(display.bounds())),
            titlebar: None,
            focus: main,
            show: true,
            kind: WindowKind::PopUp,
            is_movable: false,
            is_resizable: false,
            is_minimizable: false,
            display_id: Some(display.id()),
            ..Default::default()
        };
        let opened = gpui_kit::open_window(options, cx, move |window, cx| {
            // Closing (Command-W and the like) is refused while the ritual is due.
            window.on_window_should_close(cx, |_, _| false);
            if main {
                cx.new(|cx| RitualWindow::Main(Box::new(RitualView::new(kiosk, window, cx))))
            } else {
                cx.new(|_| RitualWindow::Elsewhere)
            }
        });
        if let Err(e) = opened {
            eprintln!("cloister-app: opening the ritual window failed: {e}");
        }
    }
    cx.activate(true);
    if kiosk.lock {
        report_lock(kiosk::lock());
    }
}

fn report_lock(options: Option<String>) {
    if let Some(options) = options {
        eprintln!("cloister-app: ritual kiosk on; system presentation options: {options}");
    }
}

pub enum RitualWindow {
    Main(Box<RitualView>),
    /// Another display: covered, and pointing at the main one.
    Elsewhere,
}

impl Render for RitualWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = cx.palette();
        let root = div()
            .size_full()
            .bg(p.ground)
            .text_color(p.ink)
            .font_family(SANS);
        match self {
            RitualWindow::Main(view) => root.child(view.render(window, cx)),
            RitualWindow::Elsewhere => root.flex().items_center().justify_center().child(
                div()
                    .flex()
                    .flex_col()
                    .gap(space::S4)
                    .items_center()
                    .child(ui::eyebrow(&p, "Shutdown Ritual"))
                    .child(
                        Type::Title
                            .apply(div())
                            .child("The ritual is on the main display."),
                    ),
            ),
        }
    }
}

/// The five steps on the main display. Lives inside [`RitualWindow::Main`], so its
/// callbacks go through the window entity.
pub struct RitualView {
    plans: Plans,
    flow: RitualFlow,
    kiosk: Kiosk,
    notice: Option<Notice>,
    loose_end: Entity<InputState>,
    typed: Entity<InputState>,
    day_plan: Option<Entity<DayPlanView>>,
    hold: Hold,
    hold_focus: FocusHandle,
    started: Instant,
    _sync: Option<SyncHandle>,
    _hold_task: Option<Task<()>>,
    _poll: Task<()>,
    _subscriptions: Vec<Subscription>,
}

type Cx<'a> = Context<'a, RitualWindow>;

fn view(this: &mut RitualWindow) -> &mut RitualView {
    match this {
        RitualWindow::Main(v) => v,
        RitualWindow::Elsewhere => unreachable!("only the main window has the steps"),
    }
}

impl RitualView {
    fn new(kiosk: Kiosk, window: &mut Window, cx: &mut Cx) -> Self {
        let plans = Plans::open();
        let sync = match &plans {
            Plans::Device(device) => Some(device.start_sync(SyncOptions::default())),
            Plans::SignedOut => None,
        };
        let loose_end =
            cx.new(|cx| InputState::new(window, cx).placeholder("Reply to Sam about Thursday"));
        let typed = cx.new(|cx| InputState::new(window, cx).placeholder("Shutdown complete"));
        let subscriptions = vec![
            cx.subscribe_in(&loose_end, window, |this, state, event, window, cx| {
                if let InputEvent::PressEnter { .. } = event {
                    let text = state.read(cx).value().to_string();
                    view(this).capture(&text, cx);
                    if view(this).notice.is_none() {
                        state.update(cx, |s, cx| s.set_value("", window, cx));
                    }
                }
            }),
            cx.subscribe_in(&typed, window, |this, state, event, _, cx| {
                if let InputEvent::Change = event {
                    let text = state.read(cx).value().to_string();
                    view(this).flow.set_typed(&text);
                    cx.notify();
                }
            }),
        ];
        loose_end.update(cx, |s, cx| s.focus(window, cx));
        let poll = cx.spawn_in(window, async move |this, cx| loop {
            cx.background_executor().timer(POLL).await;
            let alive = this.update_in(cx, |this, window, cx| view(this).poll(window, cx));
            if !matches!(alive, Ok(true)) {
                break;
            }
        });
        RitualView {
            flow: RitualFlow::open(&plans, Zone::Local),
            plans,
            kiosk,
            notice: None,
            loose_end,
            typed,
            day_plan: None,
            hold: Hold::default(),
            hold_focus: cx.focus_handle(),
            started: Instant::now(),
            _sync: sync,
            _hold_task: None,
            _poll: poll,
            _subscriptions: subscriptions,
        }
    }

    /// Once a second: quit when cloisterd no longer says the ritual is due, else re-assert the
    /// lock (something may have activated another app) and redraw. Returns whether to go on.
    fn poll(&mut self, window: &mut Window, cx: &mut Cx) -> bool {
        if let Some(after) = self.kiosk.escape_after {
            if self.started.elapsed() >= after {
                eprintln!("cloister-app: debug build, ritual kiosk lifted after {after:?}");
                quit(cx);
                return false;
            }
        }
        match SocketEnforcer::from_env().send(&Request::Status) {
            Ok(Response::Status { enforcement, .. }) if enforcement.ritual != RitualScreen::Due => {
                quit(cx);
                return false;
            }
            // Unreachable: stay. cloisterd is kept alive by launchd and decides again shortly.
            _ => {}
        }
        if self.kiosk.lock {
            kiosk::lock();
            if !cfg!(target_os = "macos") {
                kiosk::reassert(window, cfg!(target_os = "linux"));
            }
        }
        cx.notify();
        true
    }

    fn capture(&mut self, text: &str, cx: &mut Cx) {
        if text.trim().is_empty() {
            return;
        }
        self.notice = self.flow.capture(&self.plans, text);
        cx.notify();
    }

    fn close_loose_end(&mut self, id: &str, cx: &mut Cx) {
        self.notice = self.flow.close(&self.plans, id);
        cx.notify();
    }

    fn next(&mut self, window: &mut Window, cx: &mut Cx) {
        self.notice = self.flow.next(&self.plans).err();
        self.entered(window, cx);
    }

    fn back(&mut self, window: &mut Window, cx: &mut Cx) {
        self.notice = None;
        self.flow.back();
        self.entered(window, cx);
    }

    /// Set up the step just entered.
    fn entered(&mut self, window: &mut Window, cx: &mut Cx) {
        match self.flow.step() {
            RitualStep::LooseEnds => self.loose_end.update(cx, |s, cx| s.focus(window, cx)),
            RitualStep::Plan => {
                if let Some(date) = self.flow.tomorrow(&self.plans) {
                    let plan = self
                        .day_plan
                        .get_or_insert_with(|| cx.new(|cx| DayPlanView::new(window, cx)))
                        .clone();
                    plan.update(cx, |plan, cx| plan.show(Day::Date(date), cx));
                }
            }
            RitualStep::Commit => self.typed.update(cx, |s, cx| s.focus(window, cx)),
            _ => {}
        }
        cx.notify();
    }

    /// Use this week's Shutdown Delay from the ritual. cloisterd lifts the ritual on the hand-off.
    fn delay(&mut self, cx: &mut Cx) {
        let notice = use_shutdown_delay(&self.plans, &Zone::Local);
        if notice.kind == NoticeKind::Recorded {
            if let Plans::Device(device) = &self.plans {
                hand_off(device);
            }
        }
        self.notice = Some(notice);
        cx.notify();
    }

    fn change_target(&mut self, by: i32, cx: &mut Cx) {
        let next = (i32::from(self.flow.weekly_target()) + by).clamp(1, 100);
        self.flow.set_weekly_target(next as u16);
        cx.notify();
    }

    fn press_hold(&mut self, cx: &mut Cx) {
        if !self.flow.typed_matches() || self.hold.is_holding() {
            return;
        }
        self.hold.press(Instant::now());
        self._hold_task = Some(cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(16))
                .await;
            let keep_going = this.update(cx, |this, cx| {
                let this = view(this);
                if this.hold.poll(Instant::now()).is_some() {
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

    fn release_hold(&mut self, cx: &mut Cx) {
        self.hold.release();
        self._hold_task = None;
        cx.notify();
    }

    fn commit(&mut self, cx: &mut Cx) {
        match self.flow.complete(&self.plans) {
            Ok(done) => {
                self.notice = Some(done);
                // Hand the completion to cloisterd now, not at the next app sync.
                if let Plans::Device(device) = &self.plans {
                    hand_off(device);
                }
            }
            Err(notice) => self.notice = Some(notice),
        }
        cx.notify();
    }

    fn render(&mut self, _window: &mut Window, cx: &mut Cx) -> Stateful<Div> {
        let p = cx.palette();
        let step = self.flow.step();
        let delay = shutdown_delay(
            &self.plans.state().unwrap_or_default(),
            self.plans.server_time(),
        );
        let delay_button = (delay.left > 0 && matches!(self.plans, Plans::Device(_))).then(|| {
            Type::Label
                .apply(div())
                .id("delay")
                .min_h(space::TARGET)
                .px(space::S3)
                .flex()
                .items_center()
                .rounded(radius::CONTROL)
                .border_1()
                .border_color(p.ink_3)
                .cursor_pointer()
                .on_click(cx.listener(|this, _, _, cx| view(this).delay(cx)))
                .child(format!("Delay 30 min ({} left this week)", delay.left))
        });
        let all_steps = self.flow.steps().to_vec();
        let steps = all_steps.into_iter().enumerate().map(|(i, s)| {
            let on = s == step;
            div()
                .flex()
                .items_baseline()
                .gap(space::S3)
                .py(space::S2)
                .child(
                    Type::Figure(13.)
                        .apply(div())
                        .text_color(p.brass)
                        .child(format!("{:02}", i + 1)),
                )
                .child(
                    Type::Body
                        .apply(div())
                        .text_color(if on { p.ink } else { p.ink_3 })
                        .when(on, |d| d.font_weight(FontWeight::SEMIBOLD))
                        .child(s.title()),
                )
        });
        let content = match step {
            RitualStep::LooseEnds => self.loose_ends(&p, cx),
            RitualStep::Review => self.review(&p),
            RitualStep::Draft => self.draft(&p),
            RitualStep::Plan => match &self.day_plan {
                Some(plan) => div().h(px(560.)).child(plan.clone()),
                None => div(),
            },
            RitualStep::WeeklyReview => self.weekly_review(&p, cx),
            RitualStep::Commit => self.commit_step(&p, cx),
        };
        let mut nav = div().flex().gap(space::S3);
        if step != RitualStep::LooseEnds {
            nav = nav.child(
                button(&p, "back", "Back", false)
                    .on_click(cx.listener(|this, _, window, cx| view(this).back(window, cx))),
            );
        }
        if step != RitualStep::Commit {
            nav = nav.child(
                button(&p, "continue", "Continue", true)
                    .on_click(cx.listener(|this, _, window, cx| view(this).next(window, cx))),
            );
        }
        div()
            .id("ritual")
            .size_full()
            .when(cfg!(debug_assertions), |d| {
                d.on_key_down(cx.listener(|this, e: &KeyDownEvent, _, cx| {
                    if view(this).kiosk.is_escape(&e.keystroke) {
                        eprintln!("cloister-app: debug escape chord, leaving the ritual kiosk");
                        quit(cx);
                    }
                }))
            })
            .flex()
            .child(
                div()
                    .w(px(280.))
                    .h_full()
                    .flex_none()
                    .px(space::S8)
                    .pt(px(96.))
                    .bg(p.surface)
                    .border_r_1()
                    .border_color(p.line)
                    .flex()
                    .flex_col()
                    .gap(space::S2)
                    .child(ui::eyebrow(&p, "Shutdown Ritual"))
                    .children(steps)
                    .child(div().h(space::S8))
                    .children(delay_button),
            )
            .child(
                div()
                    .id("ritual-step")
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .overflow_y_scroll()
                    .child(
                        div()
                            .max_w(px(960.))
                            .px(space::S12)
                            .pt(px(96.))
                            .pb(space::S12)
                            .flex()
                            .flex_col()
                            .gap(space::S6)
                            .child(
                                div()
                                    .flex()
                                    .items_baseline()
                                    .gap(space::S4)
                                    .child(
                                        Type::Figure(20.)
                                            .apply(div())
                                            .text_color(p.brass)
                                            .child(self.flow.number()),
                                    )
                                    .child(Type::Display.apply(div()).child(step.title())),
                            )
                            .child(content)
                            .children(self.notice.clone().map(|n| ui::notice(&p, &n)))
                            .child(nav),
                    ),
            )
    }

    fn loose_ends(&mut self, p: &Palette, cx: &mut Cx) -> Div {
        let items = self.flow.loose_ends(&self.plans).into_iter().map(|l| {
            let id = l.id.clone();
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(space::S4)
                .py(space::S2)
                .border_b_1()
                .border_color(p.line)
                .child(Type::Body.apply(div()).child(l.text))
                .child(
                    Type::Label
                        .apply(div())
                        .id(SharedString::from(format!("close-{id}")))
                        .text_color(p.ink_2)
                        .underline()
                        .cursor_pointer()
                        .on_click(
                            cx.listener(move |this, _, _, cx| view(this).close_loose_end(&id, cx)),
                        )
                        .child("Close"),
                )
        });
        div()
            .flex()
            .flex_col()
            .gap(space::S4)
            .child(Type::Body.apply(div()).text_color(p.ink_2).child(
                "Write down every open task and commitment, so none stays in your head overnight. At least one tonight.",
            ))
            .child(
                div()
                    .flex()
                    .gap(space::S3)
                    .child(div().flex_1().child(Input::new(&self.loose_end)))
                    .child(button(p, "capture", "Capture", true).on_click(cx.listener(
                        |this, _, window, cx| {
                            let this = view(this);
                            let text = this.loose_end.read(cx).value().to_string();
                            this.capture(&text, cx);
                            if this.notice.is_none() {
                                this.loose_end
                                    .update(cx, |s, cx| s.set_value("", window, cx));
                            }
                        },
                    ))),
            )
            .child(ui::eyebrow(p, "Loose Ends"))
            .children(items)
    }

    fn review(&self, p: &Palette) -> Div {
        let review = self.flow.review(&self.plans);
        let rows =
            review.sessions.iter().map(|s| {
                div()
                    .flex()
                    .items_baseline()
                    .gap(space::S6)
                    .py(space::S2)
                    .border_b_1()
                    .border_color(p.line)
                    .child(
                        Type::Figure(14.)
                            .apply(div())
                            .w(px(120.))
                            .flex_none()
                            .child(s.time.clone()),
                    )
                    .child(Type::Body.apply(div()).flex_1().child(s.intent.clone()))
                    .child(Type::Label.apply(div()).text_color(p.ink_2).child(
                        match s.depth_rating {
                            Some(r) => format!("Depth Rating {r}"),
                            None => "Not rated".into(),
                        },
                    ))
            });
        div()
            .flex()
            .flex_col()
            .gap(space::S4)
            .child(ui::eyebrow(p, "Deep Sessions today"))
            .when(review.sessions.is_empty(), |d| {
                d.child(
                    Type::Body
                        .apply(div())
                        .text_color(p.ink_2)
                        .child("No Deep Session today."),
                )
            })
            .children(rows)
            .child(ui::eyebrow(p, "Deep Hours this week"))
            .child(
                Type::Figure(40.)
                    .apply(div())
                    .child(hours(review.deep_hours_this_week)),
            )
    }

    /// The week on the scoreboard, then next week's target.
    fn weekly_review(&mut self, p: &Palette, cx: &mut Cx) -> Div {
        let state = self.plans.state().unwrap_or_default();
        let board = scoreboard(&state, self.plans.server_time(), &Zone::Local);
        let step_button = |id: &'static str, label: &'static str, by: i32, cx: &mut Cx| {
            Type::Label
                .apply(div())
                .id(id)
                .h(space::TARGET)
                .px(space::S4)
                .flex()
                .items_center()
                .rounded(radius::CONTROL)
                .border_1()
                .border_color(p.ink)
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| view(this).change_target(by, cx)))
                .child(label)
        };
        div()
            .flex()
            .flex_col()
            .gap(space::S6)
            .child(crate::screens::weekly_review::scoreboard(p, &board))
            .child(ui::eyebrow(p, "Next week's target"))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(space::S3)
                    .child(step_button("target-less", "Fewer", -1, cx))
                    .child(
                        Type::Figure(28.)
                            .apply(div())
                            .w(px(160.))
                            .flex()
                            .justify_center()
                            .child(format!("{} Deep Hours", self.flow.weekly_target())),
                    )
                    .child(step_button("target-more", "More", 1, cx)),
            )
            .child(
                Type::Label
                    .apply(div())
                    .font_weight(FontWeight::NORMAL)
                    .text_color(p.ink_2)
                    .child(self.flow.target_note(&self.plans)),
            )
    }

    fn draft(&self, p: &Palette) -> Div {
        let Some(plan) = self.flow.draft(&self.plans) else {
            return div().child(
                Type::Body
                    .apply(div())
                    .text_color(p.ink_2)
                    .child("No Schedule covers tomorrow. Plan it from an empty day."),
            );
        };
        let zone = Zone::Local;
        let rows = plan.slots.iter().map(|s| {
            div()
                .flex()
                .items_baseline()
                .gap(space::S6)
                .py(space::S2)
                .border_b_1()
                .border_color(p.line)
                .child(
                    Type::Figure(14.)
                        .apply(div())
                        .w(px(120.))
                        .flex_none()
                        .child(format!(
                            "{}–{}",
                            cloister_app_model::format::clock(s.starts_at, &zone),
                            cloister_app_model::format::clock(s.ends_at, &zone)
                        )),
                )
                .child(Type::Body.apply(div()).flex_1().child(s.label.clone()))
                .when(s.deep, |d| d.child(ui::tag(p, "Deep")))
        });
        div()
            .flex()
            .flex_col()
            .gap(space::S4)
            .child(ui::eyebrow(
                p,
                &format!("{}, from its Schedule", plan.date.format("%A %-d %B")),
            ))
            .children(rows)
    }

    fn commit_step(&mut self, p: &Palette, cx: &mut Cx) -> Div {
        let enabled = self.flow.typed_matches();
        let progress = self.hold.progress(Instant::now());
        let base = div()
            .id("hold-to-commit")
            .track_focus(&self.hold_focus)
            .relative()
            .w(px(420.))
            .h(px(56.))
            .overflow_hidden()
            .rounded(radius::CONTROL)
            .flex()
            .items_center()
            .justify_center();
        let hold = if !enabled {
            base.border_1()
                .border_color(depth(4))
                .text_color(depth(2))
                .child(Type::Body.apply(div()).child("Hold to complete"))
        } else {
            base.bg(p.surface)
                .cursor_pointer()
                .focus(|s| s.border_2().border_color(depth(3)))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _: &MouseDownEvent, window, cx| {
                        let this = view(this);
                        window.focus(&this.hold_focus, cx);
                        this.press_hold(cx);
                    }),
                )
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|this, _: &MouseUpEvent, _, cx| view(this).release_hold(cx)),
                )
                .on_mouse_up_out(
                    MouseButton::Left,
                    cx.listener(|this, _: &MouseUpEvent, _, cx| view(this).release_hold(cx)),
                )
                .on_key_down(cx.listener(|this, e: &KeyDownEvent, _, cx| {
                    if matches!(e.keystroke.key.as_str(), "space" | "enter") {
                        view(this).press_hold(cx);
                        cx.stop_propagation();
                    }
                }))
                .on_key_up(cx.listener(|this, e: &KeyUpEvent, _, cx| {
                    if matches!(e.keystroke.key.as_str(), "space" | "enter") {
                        view(this).release_hold(cx);
                    }
                }))
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .w_full()
                        .h(relative(progress))
                        .bg(depth(2)),
                )
                .child(
                    Type::Body
                        .apply(div())
                        .relative()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(if self.hold.is_holding() {
                            "Keep holding"
                        } else {
                            "Hold to complete"
                        }),
                )
        };
        div()
            .flex()
            .flex_col()
            .gap(space::S4)
            .child(
                Type::Body
                    .apply(div())
                    .text_color(p.ink_2)
                    .child("Type \"Shutdown complete\", then hold for two seconds."),
            )
            .child(div().w(px(420.)).child(Input::new(&self.typed)))
            .child(hold)
    }
}

/// A plain text button: tone and a line, 48 px tall.
fn button(p: &Palette, id: &'static str, text: &'static str, primary: bool) -> Stateful<Div> {
    Type::Body
        .apply(div())
        .id(id)
        .h(space::TARGET)
        .px(space::S6)
        .flex()
        .items_center()
        .rounded(radius::CONTROL)
        .border_1()
        .border_color(if primary { depth(4) } else { p.ink })
        .when(primary, |d| d.bg(depth(4)).text_color(p.on_deep))
        .cursor_pointer()
        .child(text)
}

/// Push the account's state to cloisterd now (completion lifts the ritual there).
fn hand_off(device: &Device) {
    let _ = device.pull();
    if let Ok(request) = device.enforcer_sync() {
        let _ = SocketEnforcer::from_env().send(&request);
    }
}

fn quit(cx: &mut App) {
    kiosk::unlock();
    cx.quit();
}
