//! "The workday has ended.": Today during Evening Lockdown and on Rest Days. A restful screen in
//! the Night theme. It names the next Workday Start and links to tomorrow's Day Plan; there is
//! nothing to begin (a Deep Session can't start now).

use cloister_app_model::{View, WorkdayEnded};
use gpui_kit::*;

use super::MainView;
use crate::theme::{space, PaletteExt, Type};
use crate::ui;

pub fn render(
    this: &mut MainView,
    view: &View,
    ended: &WorkdayEnded,
    cx: &mut Context<MainView>,
) -> Div {
    let p = cx.palette();
    let notice = this.cloister.read(cx).notice.clone();
    let content = div().size_full().flex().justify_center().child(
        div()
            .w_full()
            .max_w(px(560.))
            .px(space::S12)
            .pt(px(140.))
            .flex()
            .flex_col()
            .gap(space::S6)
            .child(if ended.rest_day {
                div().flex().child(ui::tag(&p, "Rest Day"))
            } else {
                ui::eyebrow(&p, "Evening Lockdown")
            })
            .child(
                Type::Display
                    .apply(div())
                    .text_color(p.ink)
                    .child(ended.headline),
            )
            .child(
                Type::Body
                    .apply(div())
                    .text_color(p.ink_2)
                    .child(if ended.rest_day {
                        "Work tools and distractions are blocked all day."
                    } else {
                        "Work tools and distractions are blocked until the next Workday Start."
                    }),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(space::S2)
                    .child(ui::eyebrow(&p, "Next"))
                    .child(
                        Type::Heading
                            .apply(div())
                            .text_color(p.ink)
                            .child(ended.next_start_text.clone()),
                    ),
            )
            .children(notice.map(|n| ui::notice(&p, &n)))
            .child(
                Type::Body
                    .apply(div())
                    .id("tomorrows-day-plan")
                    .text_size(px(15.))
                    .text_color(p.ink_2)
                    .underline()
                    .cursor_pointer()
                    .on_click(
                        cx.listener(|this, _, window, cx| this.show_next_day_plan(window, cx)),
                    )
                    .child("Tomorrow's Day Plan"),
            ),
    );
    super::sidebar::with_sidebar(this, view, content, cx)
}
