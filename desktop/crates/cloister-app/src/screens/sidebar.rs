//! The sidebar beside every screen except the running Deep Session: the wordmark, the
//! screens, this week's Deep Hours and, when signed in, whether this Device is online.

use cloister_app_model::{format, View};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{MainView, Screen};
use crate::theme::{radius, space, PaletteExt, Type};
use crate::ui;

/// `content` with the sidebar on its left; the content scrolls.
pub fn with_sidebar(
    this: &mut MainView,
    view: &View,
    content: Div,
    cx: &mut Context<MainView>,
) -> Div {
    div()
        .size_full()
        .flex()
        .child(render(this, view, cx))
        .child(
            div()
                .id("screen-scroll")
                .flex_1()
                .min_w_0()
                .h_full()
                .overflow_y_scroll()
                .child(content),
        )
}

pub fn render(this: &mut MainView, view: &View, cx: &mut Context<MainView>) -> Div {
    let p = cx.palette();
    let sync = this.cloister.read(cx).sync_status();
    let item = |screen: Screen, label: &'static str, cx: &mut Context<MainView>| {
        let selected = this.screen == screen;
        Type::Body
            .apply(div())
            .id(label)
            .h(px(44.))
            .px(space::S3)
            .flex()
            .items_center()
            .rounded(radius::CONTROL)
            .text_size(px(15.))
            .cursor_pointer()
            .map(|d| {
                if selected {
                    d.bg(p.surface).font_weight(FontWeight::SEMIBOLD)
                } else {
                    d.text_color(p.ink_2).hover(|s| s.bg(p.surface))
                }
            })
            .on_click(cx.listener(move |this, _, _, cx| this.show(screen, cx)))
            .child(label)
    };
    div()
        .w(px(240.))
        .flex_none()
        .h_full()
        .bg(p.sunk)
        .px(px(20.))
        .py(px(28.))
        .flex()
        .flex_col()
        .gap(px(28.))
        .child(div().px(space::S3).child(ui::wordmark(&p, 26.)))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(item(Screen::Today, "Today", cx))
                .child(item(Screen::Blocklists, "Blocklists", cx))
                .child(item(Screen::Devices, "Devices", cx)),
        )
        .child(
            div()
                .px(space::S3)
                .flex()
                .flex_col()
                .gap(space::S2)
                .child(ui::eyebrow(&p, "Deep Hours"))
                .child(
                    Type::Figure(20.)
                        .apply(div())
                        .child(format::hours(view.deep_hours_this_week)),
                )
                .child(
                    Type::Label
                        .apply(div())
                        .font_weight(FontWeight::NORMAL)
                        .text_color(p.ink_2)
                        .child("this week"),
                ),
        )
        .children(view.quit_social_ends.map(|ends| {
            let zone = this.cloister.read(cx).today().zone();
            div()
                .px(space::S3)
                .flex()
                .flex_col()
                .gap(space::S2)
                .child(ui::tag(&p, "Quit-Social Block"))
                .child(
                    Type::Label
                        .apply(div())
                        .font_weight(FontWeight::NORMAL)
                        .text_color(p.ink_2)
                        .child(format!("Ends {}.", format::day(ends, &zone))),
                )
        }))
        .child(div().flex_1())
        .children(sync.map(|s| {
            div().px(space::S3).child(ui::status(
                &p,
                s.label(),
                s == cloister_sync::SyncStatus::Online,
            ))
        }))
}
