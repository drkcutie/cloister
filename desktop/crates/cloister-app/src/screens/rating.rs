//! The Depth Rating prompt: five squares on the depth ramp. It stays until answered and
//! blocks nothing: it sits on the screen, not in front of it.

use cloister_app_model::{format, RatingPrompt};
use gpui_kit::*;

use super::MainView;
use crate::theme::{depth, ink_on_depth, radius, space, PaletteExt, Type};
use crate::ui;

pub fn card(this: &mut MainView, prompt: &RatingPrompt, cx: &mut Context<MainView>) -> Div {
    let p = cx.palette();
    let zone = this.cloister.read(cx).today().zone();
    let squares = (1u8..=5).map(|level| {
        let block_id = prompt.block_id.clone();
        Type::Figure(20.)
            .apply(div())
            .id(SharedString::from(format!("depth-{level}")))
            .size(px(64.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(radius::NONE)
            .bg(depth(level))
            .text_color(ink_on_depth(level, &p))
            .cursor_pointer()
            .hover(|s| s.border_2().border_color(p.ink))
            .on_click(cx.listener(move |this, _, _, cx| {
                let id = block_id.clone();
                this.cloister.update(cx, |c, cx| c.rate(&id, level, cx));
            }))
            .child(level.to_string())
    });
    div()
        .flex()
        .flex_col()
        .gap(space::S4)
        .px(space::S6)
        .py(px(22.))
        .bg(p.surface)
        .border_1()
        .border_color(p.line)
        .rounded(radius::CARD)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(space::S1)
                .child(ui::eyebrow(&p, "Depth Rating"))
                .child(Type::Heading.apply(div()).child("How deep was it?"))
                .children(prompt.intent.clone().map(|intent| {
                    Type::Intent
                        .apply(div())
                        .text_size(px(22.))
                        .line_height(px(28.))
                        .child(intent)
                }))
                .child(
                    Type::Figure(13.)
                        .apply(div())
                        .font_weight(FontWeight::NORMAL)
                        .text_color(p.ink_2)
                        .child(format!(
                            "{} – {}",
                            format::clock(prompt.starts_at, &zone),
                            format::clock(prompt.ends_at, &zone)
                        )),
                ),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(space::S2)
                .child(div().flex().gap(space::S2).children(squares))
                .child(
                    div()
                        .w(px(64. * 5. + 8. * 4.))
                        .flex()
                        .justify_between()
                        .child(
                            Type::Label
                                .apply(div())
                                .text_color(p.ink_2)
                                .child("Shallow"),
                        )
                        .child(Type::Label.apply(div()).text_color(p.ink_2).child("Deep")),
                ),
        )
}
