//! First run: the philosophy (three screens), sign-in, installing the enforcement service with
//! one administrator prompt, the Distraction List, Workdays and times, and the first Day Plan.
//! What to show and what is allowed comes from `cloister_app_model::Onboarding`; this file only
//! draws it and runs the slow parts (email, the admin prompt) in the background.
//!
//! Shown instead of the main window until it is finished once (`OnboardingMarker`).
//! `CLOISTER_ONBOARDING=always` shows it again; `=off` skips it. Development aid:
//! `CLOISTER_ONBOARDING_STEP=sign-in|service|distraction-list|workdays` opens on that step.

use std::time::Duration;

use cloister_app_model::{
    day_name, distraction_list, install_service_command, open_account, Enforcer, Notice,
    Onboarding, OnboardingMarker, OnboardingStep, Progress, SignInForm, SocketEnforcer,
    WorkdaysForm, PHILOSOPHY,
};
use cloister_enforcer::ipc::{Request, Response};
use cloister_sync::{CommandOutcome, Device, LoopbackRedirect, Pkce};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::begin::quiet_button;
use super::day_plan::{Day, DayPlanView};
use crate::state::{Cloister, CloisterHandle};
use crate::theme::{radius, space, Palette, PaletteExt, Type, SANS};
use crate::ui;

/// How long a sign-in link stays worth waiting for.
const LINK_WAIT: Duration = Duration::from_secs(15 * 60);

/// Whether onboarding should open instead of the main window.
pub fn wanted() -> bool {
    match std::env::var("CLOISTER_ONBOARDING").as_deref() {
        Ok("off") => false,
        Ok("always") => true,
        _ => !OnboardingMarker::from_env().is_done(),
    }
}

pub struct OnboardingView {
    flow: Onboarding,
    /// This Device's account (None without a configured backend).
    device: Option<Device>,
    sign_in: SignInForm,
    sent: bool,
    workdays: WorkdaysForm,
    service_running: bool,
    installing: bool,
    notice: Option<Notice>,
    email: Entity<InputState>,
    code: Entity<InputState>,
    start: Entity<InputState>,
    shutdown: Entity<InputState>,
    day_plan: Option<Entity<DayPlanView>>,
    _task: Option<Task<()>>,
    _link: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl OnboardingView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let email = cx.new(|cx| InputState::new(window, cx).placeholder("you@example.com"));
        let code = cx.new(|cx| InputState::new(window, cx).placeholder("123456"));
        let workdays = WorkdaysForm::local();
        let start = cx.new(|cx| {
            let mut s = InputState::new(window, cx).placeholder("09:00");
            s.set_value(workdays.workday_start.clone(), window, cx);
            s
        });
        let shutdown = cx.new(|cx| {
            let mut s = InputState::new(window, cx).placeholder("17:00");
            s.set_value(workdays.shutdown_time.clone(), window, cx);
            s
        });
        let subscriptions = vec![
            cx.subscribe_in(&email, window, |this, state, event, _, cx| {
                if let InputEvent::Change = event {
                    this.sign_in.email = state.read(cx).value().to_string();
                    cx.notify();
                }
            }),
            cx.subscribe_in(&code, window, |this, state, event, _, cx| {
                if let InputEvent::Change = event {
                    this.sign_in.code = state.read(cx).value().to_string();
                    cx.notify();
                }
            }),
            cx.subscribe_in(&start, window, |this, state, event, _, cx| {
                if let InputEvent::Change = event {
                    this.workdays.workday_start = state.read(cx).value().to_string();
                    cx.notify();
                }
            }),
            cx.subscribe_in(&shutdown, window, |this, state, event, _, cx| {
                if let InputEvent::Change = event {
                    this.workdays.shutdown_time = state.read(cx).value().to_string();
                    cx.notify();
                }
            }),
        ];
        let mut flow = Onboarding::new();
        if let Ok(wanted) = std::env::var("CLOISTER_ONBOARDING_STEP") {
            let name = |s: OnboardingStep| match s {
                OnboardingStep::SignIn => "sign-in",
                OnboardingStep::Service => "service",
                OnboardingStep::DistractionList => "distraction-list",
                OnboardingStep::Workdays => "workdays",
                _ => "",
            };
            while name(flow.step()) != wanted {
                if flow.next(&Progress::default()).is_none() {
                    flow = Onboarding::new();
                    break;
                }
            }
        }
        OnboardingView {
            flow,
            device: open_account(),
            sign_in: SignInForm::default(),
            sent: false,
            workdays,
            service_running: service_running(),
            installing: false,
            notice: None,
            email,
            code,
            start,
            shutdown,
            day_plan: None,
            _task: None,
            _link: None,
            _subscriptions: subscriptions,
        }
    }

    fn progress(&self) -> Progress {
        let signed_in = self.device.as_ref().is_some_and(Device::is_signed_in);
        Progress {
            signed_in,
            service_running: self.service_running,
            settings_set: signed_in
                && self
                    .device
                    .as_ref()
                    .and_then(|d| d.state().ok())
                    .is_some_and(|s| s.settings.is_some()),
        }
    }

    fn next(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.notice = None;
        if self.flow.step() == OnboardingStep::Workdays && !self.save_settings() {
            cx.notify();
            return;
        }
        self.service_running = service_running();
        match self.flow.next(&self.progress()) {
            Some(OnboardingStep::DayPlan) => {
                let plan = cx.new(|cx| DayPlanView::new(window, cx));
                plan.update(cx, |plan, cx| plan.show(Day::Tomorrow, cx));
                self.day_plan = Some(plan);
            }
            Some(_) => {}
            None => return self.finish(window, cx),
        }
        cx.notify();
    }

    fn back(&mut self, cx: &mut Context<Self>) {
        self.notice = None;
        self.flow.back();
        cx.notify();
    }

    /// Send the Workdays settings to the account. Whether onboarding may go on.
    fn save_settings(&mut self) -> bool {
        let command = match self.workdays.command() {
            Ok(c) => c,
            Err(notice) => {
                self.notice = Some(notice);
                return false;
            }
        };
        let Some(device) = self.device.as_ref().filter(|d| d.is_signed_in()) else {
            // Nothing to save to: go on, settings can be set once signed in.
            return true;
        };
        match device.send(&command) {
            Ok(CommandOutcome::Accepted) => true,
            Ok(CommandOutcome::Rejected { reason }) => {
                self.notice = Some(Notice::refusal(
                    &reason,
                    None,
                    &cloister_app_model::Zone::Local,
                ));
                false
            }
            Err(_) => {
                self.notice = Some(Notice::what_is_so(
                    "Settings couldn't be saved: the backend isn't reachable. Try again.",
                ));
                false
            }
        }
    }

    fn finish(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Err(e) = OnboardingMarker::from_env().mark_done() {
            eprintln!("cloister-app: remembering onboarding failed: {e}");
        }
        // The account may be new since the app started: start again from it.
        let cloister = cx.new(Cloister::new);
        cx.set_global(CloisterHandle(cloister));
        window.replace_root(cx, super::MainView::new);
    }

    fn send_link(&mut self, cx: &mut Context<Self>) {
        let Some(device) = self.device.clone() else {
            return;
        };
        let email = self.sign_in.email().to_string();
        let backend = device_backend();
        self.notice = None;
        self._link = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let backend = backend.ok_or("no backend")?;
                    let pkce = Pkce::new();
                    let redirect = LoopbackRedirect::bind().map_err(|e| e.to_string())?;
                    backend
                        .send_magic_link(&email, &redirect.url(), &pkce)
                        .map_err(|e| e.to_string())?;
                    Ok::<_, String>((backend, pkce, redirect))
                })
                .await;
            let (backend, pkce, redirect) = match result {
                Ok(r) => r,
                Err(_) => {
                    let _ = this.update(cx, |this, cx| {
                        this.notice = Some(Notice::what_is_so(
                            "The email didn't go out. Check the connection and try again.",
                        ));
                        cx.notify();
                    });
                    return;
                }
            };
            let _ = this.update(cx, |this, cx| {
                this.sent = true;
                this.notice = Some(Notice::what_is_so(
                    "The email has a link and a 6-digit code. Either signs you in.",
                ));
                cx.notify();
            });
            // The link, clicked in the browser, comes back to the loopback address.
            let signed_in = cx
                .background_executor()
                .spawn(async move {
                    let code = redirect.wait_for_code(LINK_WAIT).ok()?;
                    let session = backend.exchange_code(&code, &pkce).ok()?;
                    device.sign_in(session).ok()
                })
                .await;
            if signed_in.is_some() {
                let _ = this.update_in(cx, |this, window, cx| this.signed_in(window, cx));
            }
        }));
        cx.notify();
    }

    fn verify_code(&mut self, cx: &mut Context<Self>) {
        let (Some(device), Some(backend)) = (self.device.clone(), device_backend()) else {
            return;
        };
        let email = self.sign_in.email().to_string();
        let code = self.sign_in.code.trim().to_string();
        self._task = Some(cx.spawn(async move |this, cx| {
            let ok = cx
                .background_executor()
                .spawn(async move {
                    let session = backend.verify_email_code(&email, &code).ok()?;
                    device.sign_in(session).ok()
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| match ok {
                Some(()) => this.signed_in(window, cx),
                None => {
                    this.notice = Some(Notice::what_is_so(
                        "That code didn't sign you in. Codes last one hour.",
                    ));
                    cx.notify();
                }
            });
        }));
    }

    fn signed_in(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.flow.step() == OnboardingStep::SignIn {
            self._link = None;
            self.next(window, cx);
        }
    }

    fn install(&mut self, cx: &mut Context<Self>) {
        let command = match install_service_command() {
            Ok(c) => c,
            Err(e) => {
                self.notice = Some(Notice::what_is_so(format!(
                    "The service can't be installed from here: {e}."
                )));
                cx.notify();
                return;
            }
        };
        self.installing = true;
        self.notice = None;
        self._task = Some(cx.spawn(async move |this, cx| {
            let ok = cx
                .background_executor()
                .spawn(async move {
                    let (program, args) = command;
                    std::process::Command::new(program)
                        .args(args)
                        .status()
                        .is_ok_and(|s| s.success())
                })
                .await;
            // The service takes a moment to open its socket.
            let mut running = false;
            for _ in 0..20 {
                if service_running() {
                    running = true;
                    break;
                }
                cx.background_executor()
                    .timer(Duration::from_millis(250))
                    .await;
            }
            let _ = this.update(cx, |this, cx| {
                this.installing = false;
                this.service_running = running;
                if !running {
                    this.notice = Some(Notice::what_is_so(if ok {
                        "Installed, but the service isn't answering yet. Check again in a moment."
                    } else {
                        "The service wasn't installed. The administrator prompt was closed or refused."
                    }));
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
}

fn service_running() -> bool {
    matches!(
        SocketEnforcer::from_env().send(&Request::Status),
        Ok(Response::Status { .. })
    )
}

fn device_backend() -> Option<cloister_sync::Backend> {
    cloister_sync::Backend::from_env()
}

fn primary_button(
    p: &Palette,
    id: &'static str,
    label: impl Into<SharedString>,
    enabled: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    Type::Body
        .apply(div())
        .id(id)
        .h(space::TARGET)
        .px(space::S6)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(radius::CONTROL)
        .font_weight(FontWeight::SEMIBOLD)
        .text_size(px(15.))
        .map(|d| {
            if enabled {
                d.bg(p.accent)
                    .text_color(p.on_deep)
                    .cursor_pointer()
                    .on_click(on_click)
            } else {
                d.bg(p.sunk).text_color(p.ink_3)
            }
        })
        .child(label.into())
}

fn field(p: &Palette, label: &str, input: &Entity<InputState>, width: f32) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(space::S1)
        .child(
            Type::Label
                .apply(div())
                .text_color(p.ink_2)
                .child(label.to_string()),
        )
        .child(
            div().w(px(width)).border_b_1().border_color(p.ink).child(
                Type::Body.apply(
                    Input::new(input)
                        .appearance(false)
                        .bordered(false)
                        .h(px(40.))
                        .px_0(),
                ),
            ),
        )
}

fn heading(p: &Palette, eyebrow: &str, title: &str, body: &str) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(space::S3)
        .child(ui::eyebrow(p, eyebrow))
        .child(Type::Display.apply(div()).child(title.to_string()))
        .when(!body.is_empty(), |d| {
            d.child(
                Type::Body
                    .apply(div())
                    .text_color(p.ink_2)
                    .child(body.to_string()),
            )
        })
}

impl Render for OnboardingView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = cx.palette();
        let step = self.flow.step();
        let root = div()
            .size_full()
            .bg(p.ground)
            .text_color(p.ink)
            .font_family(SANS);
        if let (OnboardingStep::DayPlan, Some(plan)) = (step, self.day_plan.clone()) {
            // The Day Plan editor takes the window, with Finish above it.
            return root.flex().flex_col().child(
                div()
                    .px(space::S12)
                    .pt(px(28.))
                    .flex()
                    .justify_between()
                    .items_end()
                    .child(heading(
                        &p,
                        "Last step",
                        "Plan tomorrow",
                        "Divide tomorrow into Slots. Deep Slots become Deep Sessions on their own.",
                    ))
                    .child(primary_button(
                        &p,
                        "finish",
                        "Finish",
                        true,
                        cx.listener(|this, _, window, cx| this.finish(window, cx)),
                    )),
            )
            .child(div().flex_1().min_h_0().child(plan));
        }

        let content = match step {
            OnboardingStep::Philosophy(n) => {
                let page = PHILOSOPHY[n];
                heading(
                    &p,
                    &format!("Why Cloister is strict, {} of {}", n + 1, PHILOSOPHY.len()),
                    page.title,
                    page.body,
                )
            }
            OnboardingStep::SignIn => self.render_sign_in(&p, cx),
            OnboardingStep::Service => self.render_service(&p, cx),
            OnboardingStep::DistractionList => render_distraction_list(&p),
            OnboardingStep::Workdays => self.render_workdays(&p, cx),
            OnboardingStep::DayPlan => div(),
        };
        let first = step == OnboardingStep::Philosophy(0);
        let can_continue = match step {
            // Signing in is the way past, unless there is no backend to sign in to.
            OnboardingStep::SignIn => self.device.is_none(),
            OnboardingStep::Service => !self.installing,
            _ => true,
        };
        let nav = div()
            .flex()
            .justify_between()
            .items_center()
            .child(if first {
                ui::wordmark(&p, 22.)
            } else {
                div().child(quiet_button(&p, "back", "Back", {
                    let view = cx.entity();
                    move |_, _, cx| view.update(cx, |this, cx| this.back(cx))
                }))
            })
            .child(primary_button(
                &p,
                "continue",
                match step {
                    OnboardingStep::Service if !self.service_running => "Not now",
                    _ => "Continue",
                },
                can_continue,
                cx.listener(|this, _, window, cx| this.next(window, cx)),
            ));
        root.flex().justify_center().child(
            div()
                .id("onboarding")
                .overflow_y_scroll()
                .w_full()
                .max_w(px(720.))
                .px(space::S12)
                .py(px(64.))
                .flex()
                .flex_col()
                .gap(px(36.))
                .child(ui::fade_in(
                    SharedString::from(format!("{step:?}")),
                    content,
                ))
                .children(self.notice.as_ref().map(|n| ui::notice(&p, n)))
                .child(nav),
        )
    }
}

impl OnboardingView {
    fn render_sign_in(&mut self, p: &Palette, cx: &mut Context<Self>) -> Div {
        let mut column = heading(
            p,
            "Sign in",
            "One account, every Device",
            "A Block started here reaches your other Devices in seconds. No password: we email \
             you a link and a code.",
        );
        if self.device.is_none() {
            return column.child(ui::notice(
                p,
                &Notice::what_is_so(
                    "This build has no backend configured (CLOISTER_API_URL, CLOISTER_ANON_KEY). \
                     Cloister works on this computer alone until it has one.",
                ),
            ));
        }
        column = column.child(field(p, "Email", &self.email, 360.)).child(
            div().flex().gap(space::S3).child(primary_button(
                p,
                "send-link",
                if self.sent {
                    "Send another email"
                } else {
                    "Send sign-in email"
                },
                self.sign_in.can_send(),
                cx.listener(|this, _, _, cx| this.send_link(cx)),
            )),
        );
        column = column.child(div().flex().child(quiet_button(
            p,
            "google",
            "Continue with Google",
            {
                let view = cx.entity();
                move |_, _, cx| view.update(cx, |this, cx| this.google(cx))
            },
        )));
        if self.sent {
            column = column.child(
                div()
                    .flex()
                    .items_end()
                    .gap(space::S4)
                    .child(field(p, "6-digit code", &self.code, 160.))
                    .child(primary_button(
                        p,
                        "verify",
                        "Sign in",
                        self.sign_in.can_verify(),
                        cx.listener(|this, _, _, cx| this.verify_code(cx)),
                    )),
            );
        }
        column
    }

    /// Google sign-in: opens the browser; the answer comes back to a loopback address, as
    /// the email link does.
    fn google(&mut self, cx: &mut Context<Self>) {
        let (Some(device), Some(backend)) = (self.device.clone(), device_backend()) else {
            return;
        };
        let pkce = Pkce::new();
        let Ok(redirect) = LoopbackRedirect::bind() else {
            self.notice = Some(Notice::what_is_so("Google sign-in couldn't start."));
            cx.notify();
            return;
        };
        cx.open_url(&backend.google_sign_in_url(&redirect.url(), &pkce));
        self._link = Some(cx.spawn(async move |this, cx| {
            let signed_in = cx
                .background_executor()
                .spawn(async move {
                    let code = redirect.wait_for_code(LINK_WAIT).ok()?;
                    let session = backend.exchange_code(&code, &pkce).ok()?;
                    device.sign_in(session).ok()
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| match signed_in {
                Some(()) => this.signed_in(window, cx),
                None => {
                    this.notice = Some(Notice::what_is_so("Google sign-in didn't complete."));
                    cx.notify();
                }
            });
        }));
    }

    fn render_service(&mut self, p: &Palette, cx: &mut Context<Self>) -> Div {
        let what = match std::env::consts::OS {
            "macos" => "macOS asks for your password once",
            "windows" => "Windows asks once (User Account Control)",
            _ => "Your system asks for your password once",
        };
        heading(
            p,
            "The enforcement service",
            "Blocks hold when the app is closed",
            &format!(
                "A small background service enforces Blocks, Evening Lockdown and the Shutdown \
                 Ritual, even after quitting the app or restarting. Installing it needs an \
                 administrator: {what}."
            ),
        )
        .child(ui::status(
            p,
            if self.service_running {
                "Running"
            } else if self.installing {
                "Installing"
            } else {
                "Not installed"
            },
            self.service_running,
        ))
        .when(!self.service_running, |d| {
            d.child(div().flex().child(primary_button(
                p,
                "install",
                "Install the service",
                !self.installing,
                cx.listener(|this, _, _, cx| this.install(cx)),
            )))
        })
    }

    fn render_workdays(&mut self, p: &Palette, cx: &mut Context<Self>) -> Div {
        let days = (1..=7u8).map(|day| {
            let on = self.workdays.workdays.contains(&day);
            div()
                .id(SharedString::from(format!("day-{day}")))
                .h(space::TARGET)
                .min_w(px(56.))
                .px(space::S3)
                .flex()
                .items_center()
                .justify_center()
                .gap(space::S2)
                .rounded(radius::CONTROL)
                .border_1()
                .border_color(p.line)
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.workdays.toggle(day);
                    cx.notify();
                }))
                .child(ui::check_square(p, on))
                .child(Type::Body.apply(div()).child(day_name(day)))
        });
        heading(
            p,
            "Workdays",
            "When work happens, and when it stops",
            "At Shutdown Time the Shutdown Ritual begins and Evening Lockdown closes work tools \
             until Workday Start. Days that aren't Workdays are Rest Days. Later, anything \
             looser takes effect the next day.",
        )
        .child(div().flex().flex_wrap().gap(space::S2).children(days))
        .child(
            div()
                .flex()
                .gap(space::S8)
                .child(field(p, "Workday Start", &self.start, 120.))
                .child(field(p, "Shutdown Time", &self.shutdown, 120.)),
        )
        .child(Type::Body.apply(div()).text_color(p.ink_2).child(format!(
            "{} Times are in {}.",
            self.workdays.summary(),
            self.workdays.time_zone
        )))
    }
}

fn render_distraction_list(p: &Palette) -> Div {
    let rows = distraction_list().into_iter().map(|category| {
        div()
            .min_h(px(56.))
            .flex()
            .items_center()
            .gap(space::S4)
            .border_b_1()
            .border_color(p.line)
            .child(ui::check_square(p, true))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(
                        Type::Body
                            .apply(div())
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(category.title),
                    )
                    .child(
                        Type::Label
                            .apply(div())
                            .text_color(p.ink_2)
                            .child(format!("{} and more", category.examples.join(", "))),
                    ),
            )
    });
    heading(
        p,
        "The Distraction List",
        "Blocked in every Deep Session",
        "Social media, news, video, shopping and forums are on from the start, on every Device. \
         Add sites and apps on the Blocklists screen at any time; switching a category off \
         takes effect the next day.",
    )
    .child(div().flex().flex_col().children(rows))
}
