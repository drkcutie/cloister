//! Small building blocks shared by the screens. All styling comes from [`crate::theme`].

use std::sync::{Arc, LazyLock};

use cloister_app_model::{Notice, NoticeKind};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::theme::{depth, motion, radius, space, Palette, Type};

static APP_ICON: LazyLock<Arc<Image>> = LazyLock::new(|| {
    Arc::new(Image::from_bytes(
        ImageFormat::Svg,
        include_bytes!("../assets/logo/app-icon.svg").to_vec(),
    ))
});

/// The app icon (the "archway of light" on its dark tile) with the wordmark.
pub fn wordmark(p: &Palette, size: f32) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .child(img(APP_ICON.clone()).size(px(30.)).flex_none())
        .child(
            Type::Title
                .apply(div())
                .text_size(px(size))
                .line_height(px(28.))
                .text_color(p.ink)
                .child("Cloister"),
        )
}

/// Section label: small brass capitals.
pub fn eyebrow(p: &Palette, text: &str) -> Div {
    Type::Eyebrow
        .apply(div())
        .text_color(p.brass)
        .child(text.to_uppercase())
}

/// A card: surface tone, 1px line, numbered heading (steps are numbered in brass).
pub fn card(p: &Palette, step: &str, heading: &str) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(space::S3)
        .px(space::S6)
        .py(px(22.))
        .bg(p.surface)
        .border_1()
        .border_color(p.line)
        .rounded(radius::CARD)
        .child(
            div()
                .flex()
                .items_baseline()
                .gap(space::S3)
                .child(
                    Type::Figure(13.)
                        .apply(div())
                        .text_color(p.brass)
                        .child(step.to_string()),
                )
                .child(
                    Type::Heading
                        .apply(div())
                        .text_color(p.ink)
                        .child(heading.to_string()),
                ),
        )
}

/// A notice: sunk for what is so, brass for what was recorded. Never an error style.
pub fn notice(p: &Palette, n: &Notice) -> Div {
    let (bg, rule) = match n.kind {
        NoticeKind::WhatIsSo => (p.sunk, p.line),
        NoticeKind::Recorded => (p.brass_tint, p.brass),
    };
    div()
        .flex()
        .items_center()
        .gap(space::S3)
        .px(space::S4)
        .py(space::S3)
        .bg(bg)
        .border_l_2()
        .border_color(rule)
        .rounded(radius::CONTROL)
        .child(
            Type::Body
                .apply(div())
                .text_color(p.ink)
                .child(n.text.clone()),
        )
}

/// Status: a word beside a square. No dots, no badges.
pub fn status(p: &Palette, word: impl Into<SharedString>, on: bool) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .child(
            div()
                .size(px(8.))
                .flex_none()
                .bg(if on { depth(4) } else { p.line }),
        )
        .child(
            Type::Label
                .apply(div())
                .text_size(px(14.))
                .text_color(p.ink)
                .child(word.into()),
        )
}

/// A tag (Deep, Quit-Social Block, ...): a label in a cut-corner box, tone and a line.
pub fn tag(p: &Palette, text: &str) -> Div {
    Type::Label
        .apply(div())
        .flex_none()
        .px(space::S2)
        .bg(p.brass_tint)
        .border_1()
        .border_color(p.brass)
        .rounded(radius::CONTROL)
        .text_color(p.ink)
        .child(text.to_string())
}

/// A square check box drawn with tone and a line (no circles, no pills).
pub fn check_square(p: &Palette, checked: bool) -> Div {
    div()
        .size(px(20.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(radius::CONTROL)
        .border_1()
        .border_color(if checked { depth(4) } else { p.ink_3 })
        .when(checked, |d| {
            d.bg(depth(4)).child(div().size(px(8.)).bg(p.on_deep))
        })
}

/// Fade a screen in when it changes (240 ms, ease-out).
pub fn fade_in(id: impl Into<ElementId>, el: Div) -> impl IntoElement {
    el.with_animation(
        id,
        Animation::new(motion::FADE).with_easing(ease_out_quint()),
        |el, t| el.opacity(t),
    )
}
