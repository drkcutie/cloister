//! The main window. One view, [`MainView`], picks the screen from the model's [`View`]:
//! Begin (with the Depth Rating prompt when one is owed) or the running Deep Session.
//!
//! To add a screen: add a module here with a `render_*` function taking `&mut MainView`
//! (or a separate `Render` entity if it owns inputs), and choose it in `MainView::render`.

mod begin;
mod rating;
mod session;

use std::time::{Duration, Instant};

use chrono::Utc;
use cloister_app_model::{BeginForm, Hold, Length};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;

use crate::state::{Cloister, CloisterHandle};
use crate::theme::{PaletteExt, SANS};
use crate::ui;

pub struct MainView {
    cloister: Entity<Cloister>,
    form: BeginForm,
    intent: Entity<InputState>,
    custom: Entity<InputState>,
    /// The last text the custom length field held that was allowed.
    custom_text: String,
    hold: Hold,
    hold_focus: FocusHandle,
    _hold_task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl MainView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let cloister = cx.global::<CloisterHandle>().0.clone();
        let intent = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Draft the methods section, start to finish.")
        });
        let custom = cx.new(|cx| InputState::new(window, cx).placeholder("75"));
        let subscriptions = vec![
            cx.observe(&cloister, |_, _, cx| cx.notify()),
            cx.subscribe_in(&intent, window, |this, state, event, _, cx| {
                if let InputEvent::Change = event {
                    this.form.intent = state.read(cx).value().to_string();
                    cx.notify();
                }
            }),
            cx.subscribe_in(&custom, window, |this, state, event, window, cx| {
                if let InputEvent::Change = event {
                    let text = state.read(cx).value().to_string();
                    if BeginForm::custom_input_allowed(&text) {
                        this.custom_text = text.clone();
                        if !text.is_empty() || matches!(this.form.length, Length::Custom(_)) {
                            this.form.length = Length::Custom(text);
                        }
                    } else {
                        // Over 24 hours (or not a number) can't be entered at all.
                        let previous = this.custom_text.clone();
                        state.update(cx, |s, cx| s.set_value(previous, window, cx));
                    }
                    cx.notify();
                }
            }),
        ];
        intent.update(cx, |s, cx| s.focus(window, cx));
        MainView {
            cloister,
            form: BeginForm::default(),
            intent,
            custom,
            custom_text: String::new(),
            hold: Hold::default(),
            hold_focus: cx.focus_handle(),
            _hold_task: None,
            _subscriptions: subscriptions,
        }
    }

    fn choose_preset(&mut self, minutes: i64, window: &mut Window, cx: &mut Context<Self>) {
        self.form.length = Length::Preset(minutes);
        self.custom_text.clear();
        self.custom.update(cx, |s, cx| s.set_value("", window, cx));
        cx.notify();
    }

    fn press_hold(&mut self, cx: &mut Context<Self>) {
        if self.form.missing().is_some() || self.hold.is_holding() {
            return;
        }
        self.hold.press(Instant::now());
        self._hold_task = Some(cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(16))
                .await;
            let keep_going = this.update(cx, |this, cx| {
                let committed = this.hold.poll(Instant::now()).is_some();
                if committed {
                    this.commit(cx);
                }
                cx.notify();
                this.hold.is_holding()
            });
            if !matches!(keep_going, Ok(true)) {
                break;
            }
        }));
        cx.notify();
    }

    fn release_hold(&mut self, cx: &mut Context<Self>) {
        self.hold.release();
        self._hold_task = None;
        cx.notify();
    }

    fn commit(&mut self, cx: &mut Context<Self>) {
        let form = self.form.clone();
        let started = self.cloister.update(cx, |c, cx| c.begin(&form, cx));
        if started {
            self.form.long_confirmed = false;
        }
    }
}

impl Render for MainView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = cx.palette();
        let view = self.cloister.read(cx).view();
        let root = div()
            .size_full()
            .bg(p.ground)
            .text_color(p.ink)
            .font_family(SANS);
        match view.active.clone() {
            Some(active) => root.child(ui::fade_in(
                SharedString::from(format!("session-{}", active.block_id)),
                session::render(self, &active, &view, window, cx),
            )),
            None => root.child(ui::fade_in("begin", begin::render(self, &view, window, cx))),
        }
    }
}

/// Open the main window, or bring it forward if it is already open.
pub fn open_main_window(cx: &mut App) {
    if let Some(window) = cx.windows().into_iter().next() {
        let _ = window.update(cx, |_, window, _| window.activate_window());
        cx.activate(true);
        return;
    }
    let bounds = Bounds::centered(None, size(px(1280.), px(800.)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        window_min_size: Some(size(px(900.), px(640.))),
        titlebar: Some(TitlebarOptions {
            title: Some("Cloister".into()),
            ..Default::default()
        }),
        ..Default::default()
    };
    if let Err(e) = gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| MainView::new(window, cx))
    }) {
        eprintln!("cloister-app: opening the window failed: {e}");
    }
    cx.activate(true);
}

/// Local time now, for screens that need it.
pub(crate) fn now() -> chrono::DateTime<Utc> {
    Utc::now()
}
