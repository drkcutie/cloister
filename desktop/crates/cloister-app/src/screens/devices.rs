//! Devices: every Device on the account with whether it is enforcing (a word beside a
//! square), and this Device's own state, "Offline, still enforcing" when it can't reach
//! the backend.

use cloister_app_model::Notice;
use gpui_kit::*;

use super::MainView;
use crate::theme::{space, PaletteExt, Type};
use crate::ui;

pub fn render(this: &mut MainView, cx: &mut Context<MainView>) -> Div {
    let p = cx.palette();
    let devices = this.cloister.read(cx).devices();
    let head = div()
        .flex()
        .flex_col()
        .gap(space::S2)
        .child(ui::eyebrow(&p, "Devices"))
        .child(
            Type::Display
                .apply(div())
                .text_size(px(48.))
                .line_height(px(52.))
                .child("Where Blocks are enforced"),
        );
    let column = div()
        .px(space::S12)
        .py(px(44.))
        .max_w(px(960.))
        .flex()
        .flex_col()
        .gap(px(28.))
        .child(head);
    let Some(view) = devices else {
        return column.child(ui::notice(
            &p,
            &Notice::what_is_so(
                "This Device isn't signed in. Sign in with cloister-device sign-in, then reopen Cloister.",
            ),
        ));
    };
    let rows = view.rows.iter().map(|row| {
        div()
            .min_h(px(56.))
            .flex()
            .flex_wrap()
            .justify_between()
            .items_center()
            .gap(space::S4)
            .border_b_1()
            .border_color(p.line)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(
                        Type::Body
                            .apply(div())
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(if row.this_device {
                                format!("{} (this Device)", row.name)
                            } else {
                                row.name.clone()
                            }),
                    )
                    .child(
                        Type::Label
                            .apply(div())
                            .font_weight(FontWeight::NORMAL)
                            .text_color(p.ink_2)
                            .child(format!("{}. {}", row.platform, row.seen)),
                    ),
            )
            .child(ui::status(&p, row.status, row.enforcing))
    });
    column.child(ui::status(&p, view.sync, view.online)).child(
        ui::card(&p, "", "Your Devices").child(div().flex().flex_col().children(rows).children(
            view.rows.is_empty().then(|| {
                Type::Body
                    .apply(div())
                    .text_color(p.ink_2)
                    .child("The list arrives once this Device reaches the backend.")
            }),
        )),
    )
}
