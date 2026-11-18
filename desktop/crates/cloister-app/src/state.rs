//! The app's one piece of shared state: [`Cloister`], an entity wrapping the model's
//! [`Today`]. The main window and the menu-bar item both read it; only the enforcement
//! service (over IPC) can change what is enforced.

use std::time::Duration;

use chrono::Utc;
use cloister_app_model::{BeginForm, Notice, SocketEnforcer, Today, View};
use gpui_kit::*;

/// How often the app asks the service for its Blocks. The countdown redraws every second
/// from the server-time offset in between.
const REFRESH: Duration = Duration::from_secs(5);

pub struct Cloister {
    today: Today<SocketEnforcer>,
    /// The last notice to show (a refusal, or what was recorded).
    pub notice: Option<Notice>,
    _ticker: Task<()>,
}

/// Global handle so the tray and any window can reach the one [`Cloister`].
pub struct CloisterHandle(pub Entity<Cloister>);
impl Global for CloisterHandle {}

impl Cloister {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let mut today = Today::new(SocketEnforcer::from_env());
        today.refresh(Utc::now());
        let ticker = cx.spawn(async move |this, cx| {
            let mut since_refresh = Duration::ZERO;
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                since_refresh += Duration::from_secs(1);
                let refresh = since_refresh >= REFRESH;
                if refresh {
                    since_refresh = Duration::ZERO;
                }
                let alive = this.update(cx, |this, cx| {
                    let now = Utc::now();
                    let before = this.today.view(now);
                    // A Block that just ended needs the service's word (and the Depth
                    // Rating prompt) straight away, not up to five seconds later.
                    let ended = before
                        .active
                        .as_ref()
                        .is_some_and(|a| a.ends_at <= this.today.server_now(now));
                    if refresh || ended || !before.reachable {
                        this.today.refresh(now);
                    }
                    cx.notify();
                });
                if alive.is_err() {
                    break;
                }
            }
        });
        Cloister {
            today,
            notice: None,
            _ticker: ticker,
        }
    }

    pub fn view(&self) -> View {
        self.today.view(Utc::now())
    }

    pub fn today(&self) -> &Today<SocketEnforcer> {
        &self.today
    }

    pub fn begin(&mut self, form: &BeginForm, cx: &mut Context<Self>) -> bool {
        let result = self.today.begin(form, Utc::now());
        self.notice = result.as_ref().err().cloned();
        cx.notify();
        result.is_ok()
    }

    pub fn rate(&mut self, block_id: &str, rating: u8, cx: &mut Context<Self>) {
        self.notice = match self.today.rate(block_id, rating, Utc::now()) {
            Ok(()) => Some(Notice::recorded(format!(
                "Depth Rating recorded: {rating}."
            ))),
            Err(notice) => Some(notice),
        };
        cx.notify();
    }

    pub fn dismiss_notice(&mut self, cx: &mut Context<Self>) {
        self.notice = None;
        cx.notify();
    }
}
