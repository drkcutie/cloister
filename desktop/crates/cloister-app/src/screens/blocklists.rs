//! Blocklists: the Distraction List categories (on by default) and the Work List, their
//! sites and apps, what is pending, and the Quit-Social Block. Additions apply now;
//! removals and switching a category off take effect tomorrow.

use cloister_app_model::{format, ListView};
use cloister_core::EntryKind;
use gpui_kit::component::input::Input;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::begin::quiet_button;
use super::MainView;
use crate::theme::{depth, radius, space, Palette, PaletteExt, Type};
use crate::ui;

pub fn render(this: &mut MainView, _window: &mut Window, cx: &mut Context<MainView>) -> Div {
    let p = cx.palette();
    let cloister = this.cloister.read(cx);
    let view = cloister.blocklists();
    let notice = cloister.blocklists_notice.clone();
    let signed_in = cloister.signed_in();
    let zone = cloister.today().zone();

    let mut column = div()
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
                .child(ui::eyebrow(&p, "Blocklists"))
                .child(
                    Type::Display
                        .apply(div())
                        .text_size(px(48.))
                        .line_height(px(52.))
                        .child("What a Block blocks"),
                )
                .child(
                    Type::Body
                        .apply(div())
                        .text_color(p.ink_2)
                        .child("Additions apply now. Removals take effect tomorrow."),
                ),
        )
        .children((!view.reachable).then(|| {
            ui::notice(
                &p,
                &cloister_app_model::Notice::what_is_so(
                    "Blocklists can't be read right now. What is known keeps being enforced.",
                ),
            )
        }))
        .children(notice.map(|n| {
            div()
                .id("blocklists-notice")
                .cursor_pointer()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.cloister.update(cx, |c, cx| c.dismiss_notice(cx));
                }))
                .child(ui::notice(&p, &n))
        }))
        .child(add_card(
            this,
            &p,
            &view.distraction,
            &view.work,
            signed_in,
            cx,
        ));

    if !view.pending.is_empty() {
        let rows = view.pending.iter().map(|change| {
            div()
                .min_h(px(44.))
                .flex()
                .flex_wrap()
                .justify_between()
                .items_center()
                .gap(space::S4)
                .border_b_1()
                .border_color(p.line)
                .child(Type::Body.apply(div()).child(change.what.clone()))
                .child(
                    Type::Label
                        .apply(div())
                        .text_color(p.ink_2)
                        .child(change.when.clone()),
                )
        });
        column = column.child(
            ui::card(&p, "", "Pending changes").child(div().flex().flex_col().children(rows)),
        );
    }

    let mut step = 1;
    for list in view.distraction.iter().chain(std::iter::once(&view.work)) {
        column = column.child(list_card(this, &p, list, &format!("{step:02}"), cx));
        step += 1;
    }

    column.child(quit_social_card(this, &p, view.quit_social_ends, &zone, cx))
}

/// Choose a Blocklist, type a site or an app, add it.
fn add_card(
    this: &mut MainView,
    p: &Palette,
    distraction: &[ListView],
    work: &ListView,
    signed_in: bool,
    cx: &mut Context<MainView>,
) -> Div {
    let choices = distraction.iter().chain(std::iter::once(work)).map(|list| {
        let selected = this.entry_list == list.id;
        let id = list.id.clone();
        Type::Body
            .apply(div())
            .id(SharedString::from(format!("choose-{}", list.id)))
            .h(px(40.))
            .px(space::S3)
            .flex()
            .items_center()
            .rounded(radius::CONTROL)
            .border_1()
            .text_size(px(15.))
            .cursor_pointer()
            .map(|d| {
                if selected {
                    d.bg(p.accent).border_color(p.accent).text_color(p.on_deep)
                } else {
                    d.border_color(p.line).hover(|s| s.border_color(depth(3)))
                }
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                this.entry_list = id.clone();
                cx.notify();
            }))
            .child(list.title.clone())
    });
    ui::card(p, "", "Add to a Blocklist")
        .child(div().flex().flex_wrap().gap(space::S2).children(choices))
        .child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(space::S2)
                .child(
                    div()
                        .flex_1()
                        .min_w(px(220.))
                        .border_b_1()
                        .border_color(p.ink)
                        .child(
                            Input::new(&this.entry)
                                .appearance(false)
                                .bordered(false)
                                .h(px(40.))
                                .px_0(),
                        ),
                )
                .child(quiet_button(
                    p,
                    "add-site",
                    "Add site",
                    cx.listener(|this, _, window, cx| this.add_entry(EntryKind::Domain, window, cx)),
                ))
                .child(quiet_button(
                    p,
                    "add-app",
                    "Add app",
                    cx.listener(|this, _, window, cx| this.add_entry(EntryKind::App, window, cx)),
                )),
        )
        .child(
            Type::Label
                .apply(div())
                .font_weight(FontWeight::NORMAL)
                .text_color(p.ink_3)
                .child(if signed_in {
                    "A site covers its subdomains. An app is its name or bundle id; it is quit whenever it opens during a Block."
                } else {
                    "Sign in on this Device to remove entries or switch categories off. Adding works without it."
                }),
        )
}

fn list_card(
    this: &mut MainView,
    p: &Palette,
    list: &ListView,
    step: &str,
    cx: &mut Context<MainView>,
) -> Div {
    let _ = this;
    let is_work = list.id == "work";
    let entry_row =
        |kind: EntryKind, entry: &cloister_app_model::EntryView, cx: &mut Context<MainView>| {
            let (list_id, value) = (list.id.clone(), entry.entry.clone());
            let pending = entry.badge.is_some();
            div()
                .min_h(px(40.))
                .flex()
                .justify_between()
                .items_center()
                .gap(space::S4)
                .border_b_1()
                .border_color(p.line)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(space::S3)
                        .child(
                            Type::Body
                                .apply(div())
                                .text_size(px(15.))
                                .child(entry.entry.clone()),
                        )
                        .children(entry.badge.clone().map(|b| ui::tag(p, &b))),
                )
                .when(!pending, |d| {
                    d.child(
                        Type::Label
                            .apply(div())
                            .id(SharedString::from(format!(
                                "remove-{}-{}",
                                list.id, entry.entry
                            )))
                            .h_full()
                            .px(space::S2)
                            .flex()
                            .items_center()
                            .text_color(p.ink_2)
                            .underline()
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let (l, v) = (list_id.clone(), value.clone());
                                this.cloister
                                    .update(cx, |c, cx| c.remove_entry(&l, kind, &v, cx));
                            }))
                            .child("Remove"),
                    )
                })
        };
    let domains: Vec<_> = list
        .domains
        .iter()
        .map(|e| entry_row(EntryKind::Domain, e, cx))
        .collect();
    let apps: Vec<_> = list
        .apps
        .iter()
        .map(|e| entry_row(EntryKind::App, e, cx))
        .collect();

    let header = div()
        .flex()
        .flex_wrap()
        .justify_between()
        .items_center()
        .gap(space::S4)
        .child(if is_work {
            ui::status(
                p,
                "Blocked during Evening Lockdown, Rest Days and when added to a Block",
                false,
            )
        } else {
            ui::status(
                p,
                if list.on { "On in every Block" } else { "Off" },
                list.on,
            )
        })
        .children(list.badge.clone().map(|b| ui::tag(p, &b)))
        .when(!is_work && list.badge.is_none(), |d| {
            let (id, on) = (list.id.clone(), list.on);
            d.child(
                Type::Label
                    .apply(div())
                    .id(SharedString::from(format!("toggle-{}", list.id)))
                    .text_color(p.ink_2)
                    .underline()
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let id = id.clone();
                        this.cloister
                            .update(cx, |c, cx| c.set_category(&id, !on, cx));
                    }))
                    .child(if on {
                        "Switch off from tomorrow"
                    } else {
                        "Switch on now"
                    }),
            )
        });

    ui::card(p, step, &list.title)
        .child(header)
        .when(!domains.is_empty(), |d| {
            d.child(ui::eyebrow(p, "Sites"))
                .child(div().flex().flex_col().children(domains))
        })
        .when(!apps.is_empty(), |d| {
            d.child(ui::eyebrow(p, "Apps"))
                .child(div().flex().flex_col().children(apps))
        })
}

fn quit_social_card(
    this: &mut MainView,
    p: &Palette,
    ends: Option<chrono::DateTime<chrono::Utc>>,
    zone: &cloister_app_model::Zone,
    cx: &mut Context<MainView>,
) -> Div {
    let card = div()
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
                .items_center()
                .gap(space::S3)
                .child(Type::Heading.apply(div()).child("Quit-Social Block"))
                .child(ui::tag(p, "30 days")),
        )
        .child(
            Type::Body
                .apply(div())
                .text_color(p.ink_2)
                .child("Thirty days without the Social list, on every Device. The one Block longer than 24 hours."),
        );
    if let Some(ends) = ends {
        return card.child(ui::status(
            p,
            format!(
                "Running. Ends {} at {}.",
                format::day(ends, zone),
                format::clock(ends, zone)
            ),
            true,
        ));
    }
    if !this.confirm_quit_social {
        return card.child(quiet_button(
            p,
            "quit-social",
            "Start a Quit-Social Block",
            cx.listener(|this, _, _, cx| {
                this.confirm_quit_social = true;
                cx.notify();
            }),
        ));
    }
    card.child(ui::notice(
        p,
        &cloister_app_model::Notice::what_is_so("30 days. Can't be undone."),
    ))
    .child(
        div()
            .flex()
            .gap(space::S2)
            .child(
                Type::Body
                    .apply(div())
                    .id("quit-social-commit")
                    .h(space::TARGET)
                    .px(space::S4)
                    .flex()
                    .items_center()
                    .rounded(radius::CONTROL)
                    .bg(depth(4))
                    .text_color(p.on_deep)
                    .font_weight(FontWeight::SEMIBOLD)
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.confirm_quit_social = false;
                        this.cloister.update(cx, |c, cx| c.start_quit_social(cx));
                    }))
                    .child("Begin the Quit-Social Block"),
            )
            .child(quiet_button(
                p,
                "quit-social-not-now",
                "Not now",
                cx.listener(|this, _, _, cx| {
                    this.confirm_quit_social = false;
                    cx.notify();
                }),
            )),
    )
}
