//! Weekly Review: the Deep Hours scoreboard. A well fills toward the weekly target, a bar per
//! day, the depth trend as squares along the Depth ramp, and the week's record (Missed Rituals,
//! Shutdown Delays used, Breaches) in brass. No percentages, no streaks. The ritual's Weekly
//! Review step draws the same [`scoreboard`].

use cloister_app_model::{scoreboard as board_of, Notice, Scoreboard};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::MainView;
use crate::theme::{depth, radius, space, Palette, PaletteExt, Type};
use crate::ui;

const WELL_HEIGHT: f32 = 220.;
const BAR_HEIGHT: f32 = 120.;

pub fn render(this: &mut MainView, cx: &mut Context<MainView>) -> Div {
    let p = cx.palette();
    let zone = this.cloister.read(cx).today().zone();
    let column = div()
        .px(space::S12)
        .py(px(44.))
        .max_w(px(960.))
        .flex()
        .flex_col()
        .gap(px(28.))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(space::S2)
                .child(ui::eyebrow(&p, "Weekly Review"))
                .child(
                    Type::Display
                        .apply(div())
                        .text_size(px(48.))
                        .line_height(px(52.))
                        .child("This week's Deep Hours"),
                ),
        );
    match this.cloister.read(cx).account() {
        Some(device) => {
            let state = device.state().unwrap_or_default();
            column.child(scoreboard(&p, &board_of(&state, device.server_time(), &zone)))
        }
        None => column.child(ui::notice(
            &p,
            &Notice::what_is_so(
                "This Device isn't signed in. Sign in with cloister-device sign-in, then reopen Cloister.",
            ),
        )),
    }
}

/// The well, the day bars, the depth trend and the record.
pub fn scoreboard(p: &Palette, board: &Scoreboard) -> Div {
    let well = div()
        .w(px(72.))
        .h(px(WELL_HEIGHT))
        .flex_none()
        .relative()
        .bg(p.sunk)
        .border_1()
        .border_color(p.line)
        .rounded(radius::CARD)
        .overflow_hidden()
        .child(
            div()
                .absolute()
                .bottom_0()
                .left_0()
                .w_full()
                .h(px(WELL_HEIGHT * board.well_fill))
                .bg(depth(4)),
        );
    let target = match board.next_week_target_hours {
        Some(next) if Some(next) != board.target_hours => {
            Some(format!("Next week's target: {next} Deep Hours."))
        }
        _ => None,
    };
    let hero = div().flex().items_end().gap(space::S8).child(well).child(
        div()
            .flex()
            .flex_col()
            .gap(space::S2)
            .child(ui::eyebrow(p, "Deep Hours"))
            .child(
                Type::Figure(40.)
                    .apply(div())
                    .child(board.hours_text.clone()),
            )
            .children(target.map(|t| {
                Type::Label
                    .apply(div())
                    .font_weight(FontWeight::NORMAL)
                    .text_color(p.ink_2)
                    .child(t)
            })),
    );
    let bars =
        div()
            .flex()
            .items_end()
            .gap(space::S4)
            .children(board.days.iter().map(|d| {
                div()
                    .w(px(56.))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(space::S2)
                    .child(Type::Figure(12.).apply(div()).text_color(p.ink_2).child(
                        if d.minutes > 0 {
                            d.text.clone()
                        } else {
                            String::new()
                        },
                    ))
                    .child(
                        div()
                            .w(px(32.))
                            .h(px(BAR_HEIGHT))
                            .flex()
                            .flex_col()
                            .justify_end()
                            .border_b_1()
                            .border_color(p.line)
                            .child(div().w_full().h(px(BAR_HEIGHT * d.fill)).bg(depth(3))),
                    )
                    .child(Type::Label.apply(div()).child(d.label.clone()))
            }));
    let trend = div()
        .flex()
        .gap(space::S1)
        .when(board.depth_trend.is_empty(), |d| {
            d.child(
                Type::Body
                    .apply(div())
                    .text_color(p.ink_2)
                    .child("No Depth Rating yet."),
            )
        })
        .children(board.depth_trend.iter().map(|&r| {
            div()
                .size(px(24.))
                .bg(depth(r))
                .flex()
                .items_center()
                .justify_center()
                .child(
                    Type::Figure(11.)
                        .apply(div())
                        .text_color(crate::theme::ink_on_depth(r, p))
                        .child(r.to_string()),
                )
        }));
    let records = board.records.iter().map(|r| {
        div()
            .flex()
            .items_center()
            .gap(space::S3)
            .py(space::S2)
            .border_b_1()
            .border_color(p.line)
            .child(ui::tag(p, r.tag))
            .child(Type::Body.apply(div()).child(r.text.clone()))
    });
    div()
        .flex()
        .flex_col()
        .gap(px(28.))
        .child(hero)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(space::S3)
                .child(ui::eyebrow(p, "By day"))
                .child(bars),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(space::S3)
                .child(ui::eyebrow(p, "Depth trend"))
                .child(trend),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(space::S2)
                .child(ui::eyebrow(p, "The week's record"))
                .when(board.records.is_empty(), |d| {
                    d.child(
                        Type::Body
                            .apply(div())
                            .text_color(p.ink_2)
                            .child("No Missed Ritual, Shutdown Delay or Breach this week."),
                    )
                })
                .children(records),
        )
}
