//! Settings: Workdays, Workday Start, Shutdown Time, the time zone (chosen once, with the first
//! settings) and the weekly Deep Hours target. Stricter changes apply now; looser ones wait,
//! and are listed with when they take effect.

use cloister_app_model::{pending_changes, Notice, SettingsForm};
use gpui_kit::component::input::Input;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::MainView;
use crate::theme::{depth, radius, space, Palette, PaletteExt, Type};
use crate::ui;

const DAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

pub fn render(this: &mut MainView, cx: &mut Context<MainView>) -> Div {
    let p = cx.palette();
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
                .child(ui::eyebrow(&p, "Settings"))
                .child(
                    Type::Display
                        .apply(div())
                        .text_size(px(48.))
                        .line_height(px(52.))
                        .child("Workdays and targets"),
                ),
        );
    let Some(device) = this.cloister.read(cx).account().cloned() else {
        return column.child(ui::notice(
            &p,
            &Notice::what_is_so(
                "This Device isn't signed in. Sign in with cloister-device sign-in, then reopen Cloister.",
            ),
        ));
    };
    let form = this
        .settings
        .get_or_insert_with(|| SettingsForm::open(&device));
    let workdays = div().flex().gap(space::S2).children((1..=7u8).map(|d| {
        let on = form.is_workday(d);
        Type::Label
            .apply(div())
            .id(SharedString::from(format!("workday-{d}")))
            .w(px(56.))
            .h(space::TARGET)
            .flex()
            .items_center()
            .justify_center()
            .rounded(radius::CONTROL)
            .border_1()
            .border_color(if on { depth(4) } else { p.line })
            .when(on, |el| el.bg(depth(4)).text_color(p.on_deep))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| {
                if let Some(f) = this.settings.as_mut() {
                    f.toggle_workday(d);
                }
                cx.notify();
            }))
            .child(DAYS[usize::from(d - 1)])
    }));
    let start = form.workday_start().to_string();
    let shutdown = form.shutdown_time().to_string();
    let target = form.weekly_target();
    let tz_editable = form.time_zone_editable();
    let tz = form.time_zone().to_string();
    let zone = this.cloister.read(cx).today().zone();
    let pending = pending_changes(
        &device.state().unwrap_or_default(),
        device.server_time(),
        &zone,
    );
    column
        .child(field(&p, "Workdays", workdays))
        .child(field(
            &p,
            "Workday Start",
            stepper(&p, "start", &start, cx, |f, by| {
                let t = SettingsForm::nudge(f.workday_start(), by);
                f.set_workday_start(&t);
            }),
        ))
        .child(field(
            &p,
            "Shutdown Time",
            stepper(&p, "shutdown", &shutdown, cx, |f, by| {
                let t = SettingsForm::nudge(f.shutdown_time(), by);
                f.set_shutdown_time(&t);
            }),
        ))
        .child(field(
            &p,
            "Time zone",
            if tz_editable {
                div().w(px(320.)).child(Input::new(&this.settings_time_zone))
            } else {
                div()
                    .flex()
                    .flex_col()
                    .gap(space::S1)
                    .child(Type::Body.apply(div()).child(tz))
                    .child(
                        Type::Label
                            .apply(div())
                            .font_weight(FontWeight::NORMAL)
                            .text_color(p.ink_2)
                            .child("Set with the first settings."),
                    )
            },
        ))
        .child(field(
            &p,
            "Weekly Deep Hours target",
            stepper(&p, "target", &format!("{target} h"), cx, |f, by| {
                let next = i64::from(f.weekly_target()) + by.signum();
                f.set_weekly_target(next.clamp(1, 100) as u16);
            }),
        ))
        .child(
            Type::Label
                .apply(div())
                .font_weight(FontWeight::NORMAL)
                .text_color(p.ink_2)
                .child(
                    "Stricter changes apply now. Looser ones take effect tomorrow; a lower target, next week.",
                ),
        )
        .children(this.settings_notice.clone().map(|n| ui::notice(&p, &n)))
        .child(
            Type::Body
                .apply(div())
                .id("save-settings")
                .h(space::TARGET)
                .px(space::S6)
                .flex()
                .items_center()
                .w(px(160.))
                .justify_center()
                .rounded(radius::CONTROL)
                .bg(depth(4))
                .text_color(p.on_deep)
                .cursor_pointer()
                .on_click(cx.listener(|this, _, _, cx| this.save_settings(cx)))
                .child("Save"),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(space::S2)
                .child(ui::eyebrow(&p, "Waiting to take effect"))
                .when(pending.is_empty(), |d| {
                    d.child(
                        Type::Body
                            .apply(div())
                            .text_color(p.ink_2)
                            .child("Nothing is waiting."),
                    )
                })
                .children(pending.into_iter().map(|line| {
                    div()
                        .py(space::S2)
                        .border_b_1()
                        .border_color(p.line)
                        .child(Type::Body.apply(div()).child(line))
                })),
        )
}

fn field(p: &Palette, label: &str, control: Div) -> Div {
    div()
        .flex()
        .items_center()
        .gap(space::S6)
        .child(
            Type::Heading
                .apply(div())
                .w(px(240.))
                .flex_none()
                .text_color(p.ink)
                .child(label.to_string()),
        )
        .child(control)
}

/// "Earlier  09:00  Later": 15-minute steps (for the target: one hour).
fn stepper(
    p: &Palette,
    id: &'static str,
    value: &str,
    cx: &mut Context<MainView>,
    change: fn(&mut SettingsForm, i64),
) -> Div {
    let button = |label: &'static str, by: i64, cx: &mut Context<MainView>| {
        Type::Label
            .apply(div())
            .id(SharedString::from(format!("{id}-{label}")))
            .h(space::TARGET)
            .px(space::S4)
            .flex()
            .items_center()
            .rounded(radius::CONTROL)
            .border_1()
            .border_color(p.ink)
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| {
                if let Some(f) = this.settings.as_mut() {
                    change(f, by);
                }
                cx.notify();
            }))
            .child(label)
    };
    div()
        .flex()
        .items_center()
        .gap(space::S3)
        .child(button("Less", -15, cx))
        .child(
            Type::Figure(20.)
                .apply(div())
                .w(px(96.))
                .flex()
                .justify_center()
                .child(value.to_string()),
        )
        .child(button("More", 15, cx))
}
