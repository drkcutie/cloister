//! The running Deep Session: the Intent as hero, a fixed-width countdown, when it ends, and
//! that it can't be cancelled. There is no stop control anywhere.

use cloister_app_model::{format, ActiveSession, View};
use gpui_kit::*;

use super::{rating, MainView};
use crate::theme::{space, PaletteExt, Type};
use crate::ui;

pub fn render(
    this: &mut MainView,
    active: &ActiveSession,
    view: &View,
    _window: &mut Window,
    cx: &mut Context<MainView>,
) -> Div {
    let p = cx.palette();
    let zone = this.cloister.read(cx).today().zone();
    let starts = format::clock(active.starts_at, &zone);
    let ends = format::clock(active.ends_at, &zone);
    let hero = match &active.intent {
        Some(intent) if active.deep => intent.clone(),
        _ => "Distraction List blocked.".to_string(),
    };
    div()
        .size_full()
        .flex()
        .flex_col()
        .child(
            div()
                .flex()
                .flex_wrap()
                .justify_between()
                .items_center()
                .gap(space::S6)
                .px(px(40.))
                .py(space::S6)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(space::S6)
                        .child(ui::wordmark(&p, 24.))
                        .child(nav_link(&p, "Blocklists", super::Screen::Blocklists, cx))
                        .child(nav_link(&p, "Devices", super::Screen::Devices, cx)),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(space::S4)
                        .child(ui::eyebrow(
                            &p,
                            if active.deep { "Deep Session" } else { "Block" },
                        ))
                        .child(
                            Type::Figure(14.)
                                .apply(div())
                                .font_weight(FontWeight::NORMAL)
                                .text_color(p.ink_2)
                                .child(format!("{starts} – {ends}")),
                        ),
                ),
        )
        .child(
            div()
                .id("session-main")
                .flex_1()
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .justify_center()
                .items_center()
                .px(px(40.))
                .pt(space::S6)
                .pb(space::S12)
                .child(
                    div()
                        .w_full()
                        .max_w(px(880.))
                        .flex()
                        .flex_col()
                        .gap(px(40.))
                        .child(
                            Type::Display
                                .apply(div())
                                .text_size(px(44.))
                                .line_height(px(52.))
                                .max_w(px(760.))
                                .child(hero),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(20.))
                                .child(
                                    Type::Timer
                                        .apply(div())
                                        .text_size(px(160.))
                                        .line_height(px(160.))
                                        .child(format::countdown(active.remaining)),
                                )
                                .child(div().h(px(6.)).w_full().bg(p.sunk).child(
                                    div().h_full().w(relative(active.progress())).bg(p.accent),
                                ))
                                .child(
                                    div()
                                        .flex()
                                        .justify_between()
                                        .gap(space::S4)
                                        .child(
                                            Type::Figure(14.)
                                                .apply(div())
                                                .font_weight(FontWeight::NORMAL)
                                                .text_color(p.ink_2)
                                                .child(starts.clone()),
                                        )
                                        .child(
                                            Type::Body
                                                .apply(div())
                                                .text_size(px(15.))
                                                .text_color(p.ink_2)
                                                .child(format!("Ends at {ends}")),
                                        ),
                                ),
                        )
                        .children(
                            view.rating
                                .as_ref()
                                .map(|prompt| rating::card(this, prompt, cx)),
                        ),
                ),
        )
        .child(
            div()
                .flex()
                .flex_wrap()
                .justify_between()
                .items_center()
                .gap(space::S6)
                .px(px(40.))
                .py(px(20.))
                .border_t_1()
                .border_color(p.line)
                .child(ui::status(
                    &p,
                    "Distraction List blocked on this Mac",
                    view.reachable,
                ))
                .child(
                    Type::Body
                        .apply(div())
                        .text_size(px(14.))
                        .text_color(p.ink_3)
                        .child("This Block can't be cancelled."),
                ),
        )
}

/// A quiet text link to another screen; the Deep Session stays the hero until it ends.
fn nav_link(
    p: &crate::theme::Palette,
    label: &'static str,
    screen: super::Screen,
    cx: &mut Context<MainView>,
) -> impl IntoElement {
    Type::Body
        .apply(div())
        .id(label)
        .text_size(px(15.))
        .text_color(p.ink_2)
        .underline()
        .cursor_pointer()
        .on_click(cx.listener(move |this, _, _, cx| this.show(screen, cx)))
        .child(label)
}
