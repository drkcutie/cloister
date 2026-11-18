//! The Day Plan editor: today's or tomorrow's Slots on a vertical timeline.
//!
//! Slots are dragged to move them and dragged by their lower edge to resize them, in steps of
//! 15 minutes. The same edits work from the keyboard on the selected Slot (Up/Down move it,
//! Shift+Up/Down change its end), and from the buttons beside the timeline. Every edit is
//! checked by the rules engine at once, so the ratchet's notice ("This Slot can be moved, not
//! removed.") appears before anything is sent. Saving goes through the backend (Day Plans are
//! account-wide), via this Device's mirror shared with `cloister-device`.

use std::path::PathBuf;

use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, Utc};
use cloister_app_model::format::clock;
use cloister_app_model::{
    DayPlanEditor, EditorSlot, Notice, NoticeKind, PlanBackend, Zone, STEP_MINUTES,
};
use cloister_core::{Command, State};
use cloister_sync::{Backend, Device, DeviceInfo, Platform};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::theme::{depth, ink_on_depth, radius, space, Palette, PaletteExt, Type};
use crate::ui;

/// Timeline scale.
const PX_PER_MINUTE: f32 = 1.2;

/// Where plans come from: a signed-in Device, or nothing (shown read-only, with a notice).
#[derive(Clone)]
enum Plans {
    Device(Device),
    SignedOut,
}

impl Plans {
    fn open() -> Self {
        let mirror = std::env::var_os("CLOISTER_MIRROR")
            .map(PathBuf::from)
            .unwrap_or_else(default_mirror);
        let device = Backend::from_env().and_then(|backend| {
            Device::open(
                backend,
                mirror,
                DeviceInfo {
                    name: "Cloister app".into(),
                    platform: Platform::current(),
                },
            )
            .ok()
        });
        match device {
            Some(d) if d.is_signed_in() => Plans::Device(d),
            _ => Plans::SignedOut,
        }
    }
}

impl PlanBackend for Plans {
    fn state(&self) -> Result<State, String> {
        match self {
            Plans::Device(d) => PlanBackend::state(d),
            Plans::SignedOut => Ok(State::default()),
        }
    }

    fn server_time(&self) -> DateTime<Utc> {
        match self {
            Plans::Device(d) => PlanBackend::server_time(d),
            Plans::SignedOut => Utc::now(),
        }
    }

    fn send(&self, command: &Command) -> Result<Option<String>, String> {
        match self {
            Plans::Device(d) => PlanBackend::send(d, command),
            Plans::SignedOut => Err("not signed in".into()),
        }
    }
}

/// The same default as `cloister-device`, so the app sees the mirror it keeps in step.
fn default_mirror() -> PathBuf {
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support/Cloister/mirror.sqlite")
    } else {
        home.join(".local/share/cloister/mirror.sqlite")
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Day {
    Today,
    Tomorrow,
}

#[derive(Clone, Copy)]
enum DragKind {
    Move,
    Resize,
}

struct Drag {
    slot: usize,
    kind: DragKind,
    from_y: Pixels,
    /// Minutes already applied during this drag.
    applied: i64,
}

pub struct DayPlanView {
    plans: Plans,
    day: Day,
    editor: DayPlanEditor,
    selected: Option<usize>,
    label: Entity<InputState>,
    notice: Option<Notice>,
    drag: Option<Drag>,
    saving: bool,
    focus: FocusHandle,
    _task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl DayPlanView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let plans = Plans::open();
        let label = cx.new(|cx| InputState::new(window, cx).placeholder("Label this Slot"));
        let subscriptions = vec![
            cx.subscribe_in(&label, window, |this, state, event, _, cx| {
                if let (InputEvent::Change, Some(i)) = (event, this.selected) {
                    let text = state.read(cx).value().to_string();
                    if this.editor.slots()[i].label != text {
                        this.editor.set_label(i, &text);
                        this.edited(cx);
                    }
                }
            }),
        ];
        let editor = DayPlanEditor::open(&plans, date_of(Day::Today), Zone::Local);
        let mut view = DayPlanView {
            plans,
            day: Day::Today,
            editor,
            selected: None,
            label,
            notice: None,
            drag: None,
            saving: false,
            focus: cx.focus_handle(),
            _task: None,
            _subscriptions: subscriptions,
        };
        view.pull(cx);
        view
    }

    /// Re-read the account in the background, then reopen the day.
    fn pull(&mut self, cx: &mut Context<Self>) {
        let Plans::Device(device) = self.plans.clone() else {
            return;
        };
        self._task = Some(cx.spawn(async move |this, cx| {
            let _ = cx
                .background_executor()
                .spawn(async move { device.pull() })
                .await;
            let _ = this.update(cx, |this, cx| this.reopen(cx));
        }));
    }

    fn reopen(&mut self, cx: &mut Context<Self>) {
        self.editor = DayPlanEditor::open(&self.plans, date_of(self.day), Zone::Local);
        self.selected = None;
        self.drag = None;
        cx.notify();
    }

    pub(crate) fn show(&mut self, day: Day, cx: &mut Context<Self>) {
        self.day = day;
        self.notice = None;
        self.reopen(cx);
    }

    fn select(&mut self, i: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        self.selected = i;
        let text = i
            .map(|i| self.editor.slots()[i].label.clone())
            .unwrap_or_default();
        self.label.update(cx, |s, cx| s.set_value(text, window, cx));
        cx.notify();
    }

    /// After any edit: say at once whether the plan would be accepted.
    fn edited(&mut self, cx: &mut Context<Self>) {
        self.notice = self.editor.check(&self.plans);
        cx.notify();
    }

    fn locked(&self, i: usize) -> bool {
        self.editor.is_locked(i, self.plans.server_time())
    }

    fn nudge(&mut self, kind: DragKind, minutes: i64, cx: &mut Context<Self>) {
        let Some(i) = self.selected else { return };
        if self.locked(i) {
            return;
        }
        match kind {
            DragKind::Move => self.editor.move_slot(i, minutes),
            DragKind::Resize => self.editor.resize_slot(i, minutes),
        }
        self.edited(cx);
    }

    fn add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let i = self.editor.add_slot();
        self.select(Some(i), window, cx);
        self.edited(cx);
        self.label.update(cx, |s, cx| s.focus(window, cx));
    }

    fn remove(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(i) = self.selected else { return };
        self.editor.remove(i);
        self.select(None, window, cx);
        self.edited(cx);
    }

    fn toggle_deep(&mut self, cx: &mut Context<Self>) {
        let Some(i) = self.selected else { return };
        if self.locked(i) {
            return;
        }
        let deep = !self.editor.slots()[i].deep;
        self.editor.set_deep(i, deep);
        self.edited(cx);
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        if let Some(n) = self.editor.check(&self.plans) {
            self.notice = Some(n);
            cx.notify();
            return;
        }
        let Plans::Device(device) = self.plans.clone() else {
            self.notice = Some(signed_out());
            cx.notify();
            return;
        };
        self.saving = true;
        let command = self.editor.command();
        cx.notify();
        self._task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { PlanBackend::send(&device, &command) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.saving = false;
                this.reopen(cx);
                this.notice = Some(match result {
                    Ok(None) => Notice::recorded(format!(
                        "Day Plan saved. Planned Deep Minutes: {}.",
                        this.editor.floor()
                    )),
                    Ok(Some(code)) => Notice::day_plan_refusal(
                        &code,
                        None,
                        this.plans.server_time(),
                        &Zone::Local,
                    ),
                    Err(_) => Notice::what_is_so(
                        "The Day Plan wasn't saved: the backend can't be reached.",
                    ),
                });
                cx.notify();
            });
        }));
    }

    fn key(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let shift = e.keystroke.modifiers.shift;
        let kind = if shift {
            DragKind::Resize
        } else {
            DragKind::Move
        };
        match e.keystroke.key.as_str() {
            "up" => self.nudge(kind, -STEP_MINUTES, cx),
            "down" => self.nudge(kind, STEP_MINUTES, cx),
            "tab" => {
                let order = self.editor.in_time_order();
                if order.is_empty() {
                    return;
                }
                let at = self
                    .selected
                    .and_then(|s| order.iter().position(|&i| i == s));
                let next = match (at, shift) {
                    (None, false) => 0,
                    (None, true) => order.len() - 1,
                    (Some(p), false) => (p + 1) % order.len(),
                    (Some(p), true) => (p + order.len() - 1) % order.len(),
                };
                self.select(Some(order[next]), window, cx);
            }
            _ => return,
        }
        cx.stop_propagation();
    }

    fn drag_to(&mut self, y: Pixels, cx: &mut Context<Self>) {
        let Some(drag) = self.drag.as_mut() else {
            return;
        };
        let minutes = f32::from(y - drag.from_y) / PX_PER_MINUTE;
        let snapped = (minutes / STEP_MINUTES as f32).round() as i64 * STEP_MINUTES;
        let delta = snapped - drag.applied;
        if delta == 0 {
            return;
        }
        drag.applied = snapped;
        let (slot, kind) = (drag.slot, drag.kind);
        match kind {
            DragKind::Move => self.editor.move_slot(slot, delta),
            DragKind::Resize => self.editor.resize_slot(slot, delta),
        }
        self.edited(cx);
    }
}

fn date_of(day: Day) -> NaiveDate {
    let today = Local::now().date_naive();
    match day {
        Day::Today => today,
        Day::Tomorrow => today + Duration::days(1),
    }
}

fn signed_out() -> Notice {
    Notice::what_is_so(
        "This Device isn't signed in. Day Plans are kept with the account: sign in with cloister-device.",
    )
}

impl Render for DayPlanView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = cx.palette();
        let date = self.editor.date();
        let title = format!("{} {} {}", date.format("%A"), date.day(), date.format("%B"));
        let signed_out = matches!(self.plans, Plans::SignedOut).then(signed_out);
        div()
            .id("day-plan")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| this.key(e, window, cx)))
            .size_full()
            .overflow_y_scroll()
            .child(
                div()
                    .px(space::S12)
                    .py(px(44.))
                    .flex()
                    .flex_col()
                    .gap(px(28.))
                    .child(
                        div()
                            .flex()
                            .items_end()
                            .justify_between()
                            .gap(space::S6)
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(space::S2)
                                    .child(ui::eyebrow(
                                        &p,
                                        if self.editor.is_saved() {
                                            "Day Plan"
                                        } else {
                                            "Day Plan, draft from the Schedule"
                                        },
                                    ))
                                    .child(
                                        Type::Display
                                            .apply(div())
                                            .text_size(px(48.))
                                            .line_height(px(52.))
                                            .child(title),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .gap(space::S2)
                                    .child(day_button(self, &p, Day::Today, "Today", cx))
                                    .child(day_button(self, &p, Day::Tomorrow, "Tomorrow", cx)),
                            ),
                    )
                    .children(signed_out.map(|n| ui::notice(&p, &n)))
                    .children(self.editor.is_rest_day().then(|| {
                        ui::notice(&p, &Notice::what_is_so("A Rest Day has no Day Plan."))
                    }))
                    .children(self.notice.clone().map(|n| {
                        div()
                            .id("plan-notice")
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.notice = None;
                                cx.notify();
                            }))
                            .child(ui::notice(&p, &n))
                    }))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap(space::S8)
                            .items_start()
                            .child(timeline(self, &p, cx))
                            .child(side(self, &p, cx)),
                    ),
            )
    }
}

fn day_button(
    this: &DayPlanView,
    p: &Palette,
    day: Day,
    text: &'static str,
    cx: &mut Context<DayPlanView>,
) -> impl IntoElement {
    let on = this.day == day;
    Type::Body
        .apply(div())
        .id(text)
        .h(space::TARGET)
        .px(space::S6)
        .flex()
        .items_center()
        .rounded(radius::CONTROL)
        .border_1()
        .border_color(if on { depth(4) } else { p.ink })
        .when(on, |d| d.bg(depth(4)).text_color(p.on_deep))
        .cursor_pointer()
        .on_click(cx.listener(move |this, _, _, cx| this.show(day, cx)))
        .child(text)
}

/// Quiet (outlined) button.
fn quiet(
    p: &Palette,
    id: &'static str,
    text: &'static str,
    enabled: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    Type::Body
        .apply(div())
        .id(id)
        .h(px(40.))
        .px(space::S4)
        .flex()
        .items_center()
        .justify_center()
        .rounded(radius::CONTROL)
        .border_1()
        .border_color(if enabled { p.ink } else { p.line })
        .text_color(if enabled { p.ink } else { p.ink_3 })
        .when(enabled, |d| d.cursor_pointer().on_click(on_click))
        .child(text)
}

fn timeline(
    this: &mut DayPlanView,
    p: &Palette,
    cx: &mut Context<DayPlanView>,
) -> impl IntoElement {
    let starts = this.editor.starts_at();
    let ends = this.editor.ends_at();
    let total = (ends - starts).num_minutes().max(1) as f32;
    let y = |t: DateTime<Utc>| px((t - starts).num_minutes() as f32 * PX_PER_MINUTE);
    let now = this.plans.server_time();

    // Hour lines, labelled on the left.
    let mut hours = vec![];
    let mut at = starts;
    while at <= ends {
        hours.push(
            div()
                .absolute()
                .top(y(at))
                .left_0()
                .right_0()
                .border_t_1()
                .border_color(p.line)
                .child(
                    Type::Figure(12.)
                        .apply(div())
                        .text_color(p.ink_3)
                        .pl(space::S1)
                        .child(clock(at, &Zone::Local)),
                ),
        );
        at += Duration::hours(1);
    }

    let slots = this.editor.slots().to_vec();
    let rows = slots.iter().enumerate().map(|(i, s)| {
        let selected = this.selected == Some(i);
        let locked = this.locked(i);
        let height = y(s.ends_at) - y(s.starts_at);
        div()
            .id(SharedString::from(format!("slot-{}", s.id)))
            .absolute()
            .top(y(s.starts_at))
            .left(px(64.))
            .right_0()
            .h(height)
            .px(space::S3)
            .py(space::S1)
            .overflow_hidden()
            .rounded(radius::CONTROL)
            .bg(if s.deep { depth(1) } else { p.surface })
            .border_1()
            .border_color(if selected { depth(3) } else { p.line })
            .when(selected, |d| d.border_2())
            .when(!locked, |d| d.cursor_grab())
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                    this.select(Some(i), window, cx);
                    window.focus(&this.focus, cx);
                    if !this.locked(i) {
                        this.drag = Some(Drag {
                            slot: i,
                            kind: DragKind::Move,
                            from_y: e.position.y,
                            applied: 0,
                        });
                    }
                    cx.stop_propagation();
                }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(space::S2)
                    .child(
                        Type::Figure(12.)
                            .apply(div())
                            .text_color(p.ink_2)
                            .child(format!(
                                "{} – {}",
                                clock(s.starts_at, &Zone::Local),
                                clock(s.ends_at, &Zone::Local)
                            )),
                    )
                    .child(
                        Type::Body
                            .apply(div())
                            .text_size(px(14.))
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(if s.label.trim().is_empty() {
                                SharedString::from("Unlabelled")
                            } else {
                                SharedString::from(s.label.clone())
                            }),
                    )
                    .when(s.deep, |d| d.child(deep_tag(p)))
                    .when(locked, |d| {
                        d.child(
                            Type::Label
                                .apply(div())
                                .text_color(p.ink_2)
                                .child(if s.ends_at <= now { "Done" } else { "Started" }),
                        )
                    }),
            )
            // Lower edge: drag to resize.
            .when(!locked, |d| {
                d.child(
                    div()
                        .id(SharedString::from(format!("resize-{}", s.id)))
                        .absolute()
                        .bottom_0()
                        .left_0()
                        .right_0()
                        .h(px(6.))
                        .cursor_row_resize()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                                this.select(Some(i), window, cx);
                                window.focus(&this.focus, cx);
                                this.drag = Some(Drag {
                                    slot: i,
                                    kind: DragKind::Resize,
                                    from_y: e.position.y,
                                    applied: 0,
                                });
                                cx.stop_propagation();
                            }),
                        ),
                )
            })
    });

    // The current time, when it falls inside the day.
    let now_line = (starts..ends).contains(&now).then(|| {
        div()
            .absolute()
            .top(y(now))
            .left(px(56.))
            .right_0()
            .h(px(2.))
            .bg(depth(4))
    });

    div()
        .w(px(380.))
        .flex_none()
        .flex()
        .flex_col()
        .gap(space::S3)
        .child(ui::eyebrow(p, "Slots"))
        .child(
            div()
                .id("timeline")
                .relative()
                .h(px(total * PX_PER_MINUTE + 16.))
                .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, _, cx| {
                    if e.pressed_button == Some(MouseButton::Left) {
                        this.drag_to(e.position.y, cx);
                    } else {
                        this.drag = None;
                    }
                }))
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|this, _: &MouseUpEvent, _, _| this.drag = None),
                )
                .on_mouse_up_out(
                    MouseButton::Left,
                    cx.listener(|this, _: &MouseUpEvent, _, _| this.drag = None),
                )
                .children(hours)
                .children(rows)
                .children(now_line),
        )
}

fn deep_tag(p: &Palette) -> Div {
    Type::Label
        .apply(div())
        .px(space::S2)
        .py(px(2.))
        .rounded(radius::CONTROL)
        .bg(depth(4))
        .text_color(ink_on_depth(4, p))
        .text_size(px(12.))
        .child("Deep")
}

fn side(this: &mut DayPlanView, p: &Palette, cx: &mut Context<DayPlanView>) -> impl IntoElement {
    let entity = cx.entity();
    let draft = this.editor.planned_deep_minutes();
    let floor = this.editor.floor();
    let selected = this.selected.map(|i| (i, this.editor.slots()[i].clone()));
    let signed_in = matches!(this.plans, Plans::Device(_));

    let add = {
        let entity = entity.clone();
        quiet(p, "add-slot", "Add a Slot", true, move |_, window, cx| {
            entity.update(cx, |this, cx| this.add(window, cx));
        })
    };

    div()
        .flex_grow(1.)
        .flex_basis(px(360.))
        .min_w(px(320.))
        .max_w(px(560.))
        .flex()
        .flex_col()
        .gap(space::S6)
        .child(match selected {
            Some((i, slot)) => slot_card(this, p, i, &slot, entity.clone()).into_any_element(),
            None => ui::card(p, "01", "Slots")
                .child(
                    Type::Body
                        .apply(div())
                        .text_color(p.ink_2)
                        .child("Select a Slot on the timeline, or add one. Drag a Slot to move it, drag its lower edge to resize it. With a Slot selected: Up and Down move it, Shift+Up and Shift+Down change its end, Tab picks the next one."),
                )
                .into_any_element(),
        })
        .child(div().flex().child(add))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(space::S2)
                .border_t_1()
                .border_color(p.line)
                .pt(space::S4)
                .child(
                    div()
                        .flex()
                        .justify_between()
                        .items_baseline()
                        .child(
                            Type::Label
                                .apply(div())
                                .text_color(p.ink_2)
                                .child("Planned Deep Minutes. Only goes up."),
                        )
                        .child(Type::Figure(14.).apply(div()).child(draft.to_string())),
                )
                .when(floor > 0, |d| {
                    d.child(
                        Type::Label
                            .apply(div())
                            .font_weight(FontWeight::NORMAL)
                            .text_color(p.ink_3)
                            .child(format!("Today's floor: {floor}. Deep Slots can move; this can't go below it.")),
                    )
                }),
        )
        .child(save_button(this, p, signed_in, cx))
}

fn slot_card(
    this: &mut DayPlanView,
    p: &Palette,
    i: usize,
    slot: &EditorSlot,
    entity: Entity<DayPlanView>,
) -> Div {
    let locked = this.locked(i);
    let step = |id: &'static str, text: &'static str, kind: DragKind, minutes: i64| {
        let entity = entity.clone();
        quiet(p, id, text, !locked, move |_, _, cx| {
            entity.update(cx, |this, cx| this.nudge(kind, minutes, cx));
        })
    };
    let deep_toggle = {
        let entity = entity.clone();
        div()
            .id("slot-deep")
            .h(space::TARGET)
            .flex()
            .items_center()
            .gap(space::S3)
            .when(!locked, |d| {
                d.cursor_pointer().on_click(move |_, _, cx| {
                    entity.update(cx, |this, cx| this.toggle_deep(cx));
                })
            })
            .child(ui::check_square(p, slot.deep))
            .child(Type::Body.apply(div()).child("Deep"))
            .child(
                Type::Label
                    .apply(div())
                    .font_weight(FontWeight::NORMAL)
                    .text_color(p.ink_3)
                    .child(if slot.deep {
                        "Becomes a Deep Session at its start. The label is its Intent."
                    } else {
                        "A label only. Nothing is blocked."
                    }),
            )
    };
    let remove = {
        let entity = entity.clone();
        Type::Body
            .apply(div())
            .id("slot-remove")
            .underline()
            .text_color(if locked { p.ink_3 } else { p.ink })
            .when(!locked, |d| {
                d.cursor_pointer().on_click(move |_, window, cx| {
                    entity.update(cx, |this, cx| this.remove(window, cx));
                })
            })
            .child("Take off the plan")
    };
    ui::card(
        p,
        "01",
        &format!(
            "{} – {}",
            clock(slot.starts_at, &Zone::Local),
            clock(slot.ends_at, &Zone::Local)
        ),
    )
    .when(locked, |d| {
        d.child(ui::notice(
            p,
            &Notice {
                kind: NoticeKind::WhatIsSo,
                text: format!(
                    "This Deep Session has started. Ends at {}. Can't be changed.",
                    clock(slot.ends_at, &Zone::Local)
                ),
            },
        ))
    })
    .child(
        div()
            .h(px(44.))
            .border_b_1()
            .border_color(p.ink)
            .flex()
            .items_end()
            .child(
                Input::new(&this.label)
                    .appearance(false)
                    .bordered(false)
                    .px_0()
                    .disabled(locked),
            ),
    )
    .child(deep_toggle)
    .child(
        div()
            .flex()
            .flex_wrap()
            .gap(space::S2)
            .child(step("earlier", "Earlier", DragKind::Move, -STEP_MINUTES))
            .child(step("later", "Later", DragKind::Move, STEP_MINUTES))
            .child(step("shorter", "Shorter", DragKind::Resize, -STEP_MINUTES))
            .child(step("longer", "Longer", DragKind::Resize, STEP_MINUTES)),
    )
    .child(div().flex().child(remove))
}

fn save_button(
    this: &DayPlanView,
    p: &Palette,
    signed_in: bool,
    cx: &mut Context<DayPlanView>,
) -> impl IntoElement {
    let enabled = signed_in && !this.saving;
    Type::Body
        .apply(div())
        .id("save-plan")
        .h(space::TARGET)
        .px(space::S6)
        .flex()
        .items_center()
        .justify_center()
        .rounded(radius::CONTROL)
        .font_weight(FontWeight::SEMIBOLD)
        .bg(if enabled { depth(4) } else { p.sunk })
        .text_color(if enabled { p.on_deep } else { p.ink_3 })
        .when(enabled, |d| {
            d.cursor_pointer()
                .on_click(cx.listener(|this, _, _, cx| this.save(cx)))
        })
        .child(if this.saving {
            "Saving"
        } else {
            "Save Day Plan"
        })
}
