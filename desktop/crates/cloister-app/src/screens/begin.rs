//! Begin a Deep Session (or, with "counts as deep work" off, a plain Block).

use cloister_app_model::{format, BeginForm, Length, Missing, Notice, View};
use cloister_core::EntryKind;
use gpui_kit::component::input::Input;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{now, rating, MainView};
use crate::theme::{depth, radius, space, Palette, PaletteExt, Type};
use crate::ui;

pub fn render(
    this: &mut MainView,
    view: &View,
    window: &mut Window,
    cx: &mut Context<MainView>,
) -> Div {
    let content = main_column(this, view, window, cx);
    super::sidebar::with_sidebar(this, view, content, cx)
}

fn main_column(
    this: &mut MainView,
    view: &View,
    window: &mut Window,
    cx: &mut Context<MainView>,
) -> Div {
    let p = cx.palette();
    let deep = this.form.deep;
    let notice = cx
        .global::<crate::state::CloisterHandle>()
        .0
        .read(cx)
        .notice
        .clone();
    let unreachable = (!view.reachable).then(Notice::unreachable);

    div()
        .px(space::S12)
        .py(px(44.))
        .flex()
        .flex_col()
        .gap(px(28.))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(space::S2)
                .child(ui::eyebrow(
                    &p,
                    if deep {
                        "New Deep Session"
                    } else {
                        "New Block"
                    },
                ))
                .child(
                    Type::Display
                        .apply(div())
                        .text_size(px(48.))
                        .line_height(px(52.))
                        .child(if deep {
                            "Begin a Deep Session"
                        } else {
                            "Begin a Block"
                        }),
                ),
        )
        .children(unreachable.map(|n| ui::notice(&p, &n)))
        .children(notice.map(|n| {
            div()
                .id("notice")
                .cursor_pointer()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.cloister.update(cx, |c, cx| c.dismiss_notice(cx));
                }))
                .child(ui::notice(&p, &n))
        }))
        .children(
            view.rating
                .as_ref()
                .map(|prompt| rating::card(this, prompt, cx)),
        )
        .child(
            div()
                .flex()
                .flex_wrap()
                .gap(space::S6)
                .items_start()
                .child(
                    div()
                        .flex_grow(999.)
                        .flex_basis(px(420.))
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(space::S4)
                        .child(deep_toggle(this, &p, cx))
                        .when(deep, |d| d.child(intent_card(this, &p)))
                        .child(length_card(this, &p, window, cx))
                        .child(blocklist_card(this, &p, if deep { "03" } else { "02" }, cx)),
                )
                .child(summary_band(this, view, &p, cx)),
        )
}

fn deep_toggle(this: &mut MainView, p: &Palette, cx: &mut Context<MainView>) -> impl IntoElement {
    let on = this.form.deep;
    div()
        .id("deep-toggle")
        .h(space::TARGET)
        .px(space::S1)
        .flex()
        .items_center()
        .gap(space::S3)
        .cursor_pointer()
        .on_click(cx.listener(|this, _, _, cx| {
            this.form.deep = !this.form.deep;
            cx.notify();
        }))
        .child(ui::check_square(p, on))
        .child(Type::Body.apply(div()).child("Counts as deep work"))
        .child(
            Type::Label
                .apply(div())
                .font_weight(FontWeight::NORMAL)
                .text_color(p.ink_3)
                .child(if on {
                    "A Deep Session: Intent, Depth Rating, Deep Hours."
                } else {
                    "A plain Block: no Intent, no Deep Hours."
                }),
        )
}

fn intent_card(this: &mut MainView, p: &Palette) -> Div {
    ui::card(p, "01", "Intent")
        .child(
            div()
                .h(px(56.))
                .border_b_1()
                .border_color(p.ink)
                .flex()
                .items_end()
                .child(
                    Type::Intent.apply(
                        Input::new(&this.intent)
                            .appearance(false)
                            .bordered(false)
                            .h(px(52.))
                            .px_0(),
                    ),
                ),
        )
        .child(
            Type::Label
                .apply(div())
                .font_weight(FontWeight::NORMAL)
                .text_color(p.ink_3)
                .child("What will you produce?"),
        )
}

fn length_card(
    this: &mut MainView,
    p: &Palette,
    window: &mut Window,
    cx: &mut Context<MainView>,
) -> Div {
    let _ = window;
    let step = if this.form.deep { "02" } else { "01" };
    let custom_selected = matches!(this.form.length, Length::Custom(_));
    let presets = BeginForm::PRESETS.iter().map(|&m| {
        let selected = this.form.length == Length::Preset(m);
        Type::Figure(15.)
            .apply(div())
            .id(SharedString::from(format!("preset-{m}")))
            .h(space::TARGET)
            .min_w(px(88.))
            .px(space::S4)
            .flex()
            .items_center()
            .justify_center()
            .rounded(radius::CONTROL)
            .border_1()
            .cursor_pointer()
            .map(|d| {
                if selected {
                    d.bg(p.accent).border_color(p.accent).text_color(p.on_deep)
                } else {
                    d.border_color(p.line)
                        .font_weight(FontWeight::NORMAL)
                        .hover(|s| s.border_color(depth(3)))
                }
            })
            .on_click(cx.listener(move |this, _, window, cx| this.choose_preset(m, window, cx)))
            .child(format!("{m}m"))
    });
    let custom = div()
        .h(space::TARGET)
        .px(px(14.))
        .flex()
        .items_center()
        .gap(space::S2)
        .rounded(radius::CONTROL)
        .border_1()
        .border_dashed()
        .border_color(if custom_selected { depth(4) } else { p.ink_3 })
        .child(
            Type::Body
                .apply(div())
                .text_size(px(15.))
                .text_color(p.ink_2)
                .child("Custom"),
        )
        .child(
            div().w(px(56.)).border_b_1().border_color(p.ink).child(
                Type::Figure(15.).apply(
                    Input::new(&this.custom)
                        .appearance(false)
                        .bordered(false)
                        .h(px(32.))
                        .px_0(),
                ),
            ),
        )
        .child(Type::Figure(15.).apply(div()).child("m"));

    let mut card = ui::card(p, step, "Length").child(
        div()
            .flex()
            .flex_wrap()
            .gap(space::S2)
            .children(presets)
            .child(custom),
    );
    if this.form.is_long() {
        let confirmed = this.form.long_confirmed;
        card = card.child(
            div()
                .id("confirm-long")
                .min_h(space::TARGET)
                .px(space::S4)
                .py(space::S3)
                .flex()
                .items_center()
                .gap(space::S3)
                .bg(p.sunk)
                .rounded(radius::CONTROL)
                .cursor_pointer()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.form.long_confirmed = !this.form.long_confirmed;
                    cx.notify();
                }))
                .child(ui::check_square(p, confirmed))
                .child(
                    Type::Body
                        .apply(div())
                        .child("Over 4 hours. I understand this Block can't be undone."),
                ),
        );
    }
    card.child(
        Type::Label
            .apply(div())
            .font_weight(FontWeight::NORMAL)
            .text_color(p.ink_3)
            .child("Up to 24 hours."),
    )
}

fn blocklist_card(this: &mut MainView, p: &Palette, step: &str, cx: &mut Context<MainView>) -> Div {
    let extras = this
        .form
        .extra_domains
        .iter()
        .map(|d| (EntryKind::Domain, d.clone()))
        .chain(
            this.form
                .extra_apps
                .iter()
                .map(|a| (EntryKind::App, a.clone())),
        )
        .map(|(kind, entry)| {
            let id = SharedString::from(format!("extra-{entry}"));
            let shown = entry.clone();
            div()
                .h(px(36.))
                .pl(space::S3)
                .flex()
                .items_center()
                .gap(space::S2)
                .bg(p.surface)
                .border_1()
                .border_color(p.line)
                .rounded(radius::CONTROL)
                .child(Type::Body.apply(div()).text_size(px(15.)).child(shown))
                .child(
                    Type::Label
                        .apply(div())
                        .id(id)
                        .h_full()
                        .px(space::S3)
                        .flex()
                        .items_center()
                        .text_color(p.ink_2)
                        .underline()
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.form.remove_extra(kind, &entry);
                            cx.notify();
                        }))
                        .child("Remove"),
                )
        })
        .collect::<Vec<_>>();
    ui::card(p, step, "Blocklist")
        .child(
            div()
                .flex()
                .flex_wrap()
                .justify_between()
                .items_center()
                .gap(space::S4)
                .px(px(14.))
                .py(space::S3)
                .bg(depth(1))
                .rounded(radius::CONTROL)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .child(
                            Type::Body
                                .apply(div())
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("Distraction List"),
                        )
                        .child(
                            Type::Label
                                .apply(div())
                                .font_weight(FontWeight::NORMAL)
                                .text_color(p.ink)
                                .child("Social, news, video, shopping, forums"),
                        ),
                )
                .child(
                    Type::Label
                        .apply(div())
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Always on"),
                ),
        )
        .child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(space::S2)
                .child(
                    div()
                        .flex_1()
                        .min_w(px(200.))
                        .border_b_1()
                        .border_color(p.ink)
                        .child(
                            Input::new(&this.extra)
                                .appearance(false)
                                .bordered(false)
                                .h(px(40.))
                                .px_0(),
                        ),
                )
                .child(quiet_button(
                    p,
                    "add-extra-site",
                    "Add site",
                    cx.listener(|this, _, window, cx| {
                        this.add_extra(EntryKind::Domain, window, cx)
                    }),
                ))
                .child(quiet_button(
                    p,
                    "add-extra-app",
                    "Add app",
                    cx.listener(|this, _, window, cx| this.add_extra(EntryKind::App, window, cx)),
                )),
        )
        .when(!extras.is_empty(), |d| {
            d.child(div().flex().flex_wrap().gap(space::S2).children(extras))
        })
        .child(
            Type::Label
                .apply(div())
                .font_weight(FontWeight::NORMAL)
                .text_color(p.ink_3)
                .child("Extra sites and apps are blocked for this Block only."),
        )
}

/// A Quiet button: outlined, 48px tall.
pub(crate) fn quiet_button(
    p: &Palette,
    id: &'static str,
    label: &'static str,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    Type::Body
        .apply(div())
        .id(id)
        .h(space::TARGET)
        .px(space::S4)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(radius::CONTROL)
        .border_1()
        .border_color(p.ink_3)
        .font_weight(FontWeight::SEMIBOLD)
        .text_size(px(15.))
        .cursor_pointer()
        .hover(|s| s.border_color(depth(3)))
        .on_click(on_click)
        .child(label)
}

fn summary_band(this: &mut MainView, view: &View, p: &Palette, cx: &mut Context<MainView>) -> Div {
    let cloister = this.cloister.read(cx);
    let zone = cloister.today().zone();
    let summary = cloister.today().summary(&this.form, now());
    let row = |label: &str, value: String| {
        div()
            .h(px(44.))
            .flex()
            .justify_between()
            .items_center()
            .border_b_1()
            .border_color(depth(4))
            .child(
                Type::Body
                    .apply(div())
                    .text_size(px(15.))
                    .text_color(depth(1))
                    .child(label.to_string()),
            )
            .child(Type::Figure(14.).apply(div()).child(value))
    };
    let dash = || "–".to_string();
    let missing = if view.reachable {
        this.form.missing()
    } else {
        None
    };
    div()
        .flex_grow(1.)
        .flex_basis(px(300.))
        .bg(depth(5))
        .text_color(p.on_deep)
        .rounded(radius::CARD)
        .px(px(28.))
        .pt(px(26.))
        .pb(px(28.))
        .flex()
        .flex_col()
        .gap(px(20.))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(ui::eyebrow(p, "This Block").text_color(p.brass_on_deep))
                .child(
                    Type::Figure(40.).apply(div()).line_height(px(44.)).child(
                        summary
                            .as_ref()
                            .map_or_else(dash, |s| format::hours(s.length)),
                    ),
                ),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .border_t_1()
                .border_color(depth(4))
                .child(row(
                    "Starts",
                    summary
                        .as_ref()
                        .map_or_else(dash, |s| format::clock(s.starts_at, &zone)),
                ))
                .child(row(
                    "Ends",
                    summary
                        .as_ref()
                        .map_or_else(dash, |s| format::clock(s.ends_at, &zone)),
                ))
                .child(row(
                    "Deep Hours after",
                    summary
                        .as_ref()
                        .map_or_else(dash, |s| format::hours(s.deep_hours_after)),
                )),
        )
        .child(
            Type::Body
                .apply(div())
                .text_size(px(15.))
                .line_height(px(23.))
                .text_color(depth(1))
                .child(match missing {
                    Some(Missing::Intent) => "Write an Intent to begin.",
                    Some(Missing::Length) => "Choose a length to begin.",
                    Some(Missing::LongConfirmation) => "Confirm that this Block can't be undone.",
                    None => "Can't be cancelled once begun.",
                }),
        )
        .child(hold_button(
            this,
            p,
            view.reachable && missing.is_none(),
            cx,
        ))
}

/// Hold to begin: two seconds, with the pointer or a held Space or Return.
fn hold_button(
    this: &mut MainView,
    p: &Palette,
    enabled: bool,
    cx: &mut Context<MainView>,
) -> impl IntoElement {
    let progress = this.hold.progress(std::time::Instant::now());
    let holding = this.hold.is_holding();
    let label = if !enabled {
        "Hold to begin"
    } else if holding {
        "Keep holding"
    } else {
        "Hold to begin"
    };
    let base = div()
        .id("hold-to-begin")
        .track_focus(&this.hold_focus)
        .relative()
        .w_full()
        .h(px(56.))
        .overflow_hidden()
        .rounded(radius::CONTROL)
        .flex()
        .items_center()
        .justify_center();
    if !enabled {
        return base
            .border_1()
            .border_color(depth(4))
            .text_color(depth(2))
            .child(
                Type::Body
                    .apply(div())
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(label),
            );
    }
    base.bg(p.surface)
        .text_color(p.ink)
        .cursor_pointer()
        .focus(|s| s.border_2().border_color(depth(3)))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _: &MouseDownEvent, window, cx| {
                window.focus(&this.hold_focus, cx);
                this.press_hold(cx);
            }),
        )
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(|this, _: &MouseUpEvent, _, cx| this.release_hold(cx)),
        )
        .on_mouse_up_out(
            MouseButton::Left,
            cx.listener(|this, _: &MouseUpEvent, _, cx| this.release_hold(cx)),
        )
        .on_key_down(cx.listener(|this, e: &KeyDownEvent, _, cx| {
            if matches!(e.keystroke.key.as_str(), "space" | "enter") {
                this.press_hold(cx);
                cx.stop_propagation();
            }
        }))
        .on_key_up(cx.listener(|this, e: &KeyUpEvent, _, cx| {
            if matches!(e.keystroke.key.as_str(), "space" | "enter") {
                this.release_hold(cx);
            }
        }))
        // The fill sinks from the top over the two seconds, linearly.
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
                .child(label),
        )
}
