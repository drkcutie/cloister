//! The app's one piece of shared state: [`Cloister`], an entity wrapping the model's
//! [`Today`]. The main window and the menu-bar item both read it; only the enforcement
//! service (over IPC) can change what is enforced.

use std::time::Duration;

use chrono::Utc;
use cloister_app_model::{
    devices_view, open_account, shutdown_delay, shutdown_warning, AccountRules, BeginForm,
    Blocklists, BlocklistsView, DelayView, DevicesView, Enforcer, Notice, Rules, ServiceRules,
    SocketEnforcer, Today, View,
};
use cloister_core::EntryKind;
use cloister_enforcer::ipc::{Planning, Request};
use cloister_sync::{Device, DeviceSummary, SyncHandle, SyncOptions, SyncStatus};
use gpui_kit::*;

/// How often the app asks the service for its Blocks. The countdown redraws every second
/// from the server-time offset in between.
const REFRESH: Duration = Duration::from_secs(5);

/// How often the Devices list is re-read from the backend.
const DEVICES_EVERY: Duration = Duration::from_secs(15);

pub struct Cloister {
    today: Today<SocketEnforcer>,
    /// The last notice to show (a refusal, or what was recorded).
    pub notice: Option<Notice>,
    blocklists: Blocklists<Box<dyn Rules>>,
    /// The last notice from the Blocklists screen.
    pub blocklists_notice: Option<Notice>,
    /// This Device's account, when the backend is configured and it is signed in.
    account: Option<Device>,
    devices: Vec<DeviceSummary>,
    _sync: Option<SyncHandle>,
    _ticker: Task<()>,
    /// Whether the app wears Night (Evening Lockdown, Rest Days), and the settle under way.
    night: bool,
    _settle: Option<Task<()>>,
    /// The last Shutdown Time warning shown: (due at, minutes before).
    warned: Option<(chrono::DateTime<Utc>, i64)>,
}

/// Global handle so the tray and any window can reach the one [`Cloister`].
pub struct CloisterHandle(pub Entity<Cloister>);
impl Global for CloisterHandle {}

impl Cloister {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let mut today = Today::new(SocketEnforcer::from_env());
        today.refresh(Utc::now());
        // Opened at night: already Night, no settle.
        let night = today.view(Utc::now()).night();
        if night {
            crate::theme::use_palette(cx, crate::theme::Palette::night());
        }
        let account = open_account().filter(Device::is_signed_in);
        // Blocklist edits go to the account when signed in (it decides; loosening waits
        // until tomorrow), else to the local service, which only takes tightening edits.
        let rules: Box<dyn Rules> = match &account {
            Some(device) => Box::new(AccountRules(device.clone())),
            None => Box::new(ServiceRules(SocketEnforcer::from_env())),
        };
        let mut blocklists = Blocklists::new(rules);
        blocklists.refresh();
        let sync = account
            .as_ref()
            .map(|device| device.start_sync(SyncOptions::default()));
        let ticker = cx.spawn(async move |this, cx| {
            let mut since_refresh = Duration::ZERO;
            let mut since_devices = DEVICES_EVERY;
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                since_refresh += Duration::from_secs(1);
                let refresh = since_refresh >= REFRESH;
                if refresh {
                    since_refresh = Duration::ZERO;
                }
                since_devices += Duration::from_secs(1);
                if since_devices >= DEVICES_EVERY {
                    since_devices = Duration::ZERO;
                    let account = this.update(cx, |this, _| this.account.clone());
                    if let Ok(Some(device)) = account {
                        let listed = cx.background_spawn(async move { device.devices() }).await;
                        if let Ok(devices) = listed {
                            let _ = this.update(cx, |this, cx| {
                                this.devices = devices;
                                cx.notify();
                            });
                        }
                    }
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
                        this.hand_off_to_service();
                        this.blocklists.refresh();
                    }
                    this.warn_before_shutdown();
                    // Shutdown Time and Workday Start: settle into Night or back to Day.
                    let night = this.today.view(now).night();
                    if night != this.night {
                        this.night = night;
                        this._settle = Some(crate::theme::settle(cx, night));
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
            blocklists,
            blocklists_notice: None,
            account,
            devices: Vec::new(),
            _sync: sync,
            _ticker: ticker,
            night,
            _settle: None,
            warned: None,
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
        self.blocklists_notice = None;
        cx.notify();
    }

    /// Hand what the account knows to the enforcement service (tighten-only on its side),
    /// and tell the account whether this Device is enforcing.
    fn hand_off_to_service(&mut self) {
        let Some(device) = &self.account else { return };
        let Ok(state) = device.state() else { return };
        let reached = SocketEnforcer::from_env()
            .send(&Request::Sync {
                blocks: state.blocks.clone(),
                blocklists: Some(state.blocklist_edits()),
                planning: Some(Planning::of(&state)),
            })
            .is_ok();
        let device = device.clone();
        // A network call: off the main thread.
        std::thread::spawn(move || {
            let _ = device.report_enforcing(reached);
        });
    }

    /// Shutdown Time warnings, 15 and 5 minutes before (later with a Shutdown Delay): a
    /// system notification, once each.
    fn warn_before_shutdown(&mut self) {
        let Some(device) = &self.account else { return };
        let Ok(state) = device.state() else { return };
        let Some(warning) = shutdown_warning(&state, device.server_time(), &self.today.zone())
        else {
            return;
        };
        let key = (warning.due_at, warning.minutes);
        if self.warned != Some(key) {
            self.warned = Some(key);
            notify(&warning.text);
        }
    }

    /// Shutdown Delays left this week, when signed in (a delay goes through the account).
    pub fn shutdown_delay(&self) -> Option<DelayView> {
        let device = self.account.as_ref()?;
        Some(shutdown_delay(&device.state().ok()?, device.server_time()))
    }

    /// Use this week's Shutdown Delay (from the menu bar).
    pub fn use_shutdown_delay(&mut self, cx: &mut Context<Self>) {
        let Some(device) = &self.account else { return };
        let notice = cloister_app_model::use_shutdown_delay(device, &self.today.zone());
        notify(&notice.text);
        self.notice = Some(notice);
        self.hand_off_to_service();
        cx.notify();
    }

    pub fn blocklists(&self) -> BlocklistsView {
        self.blocklists.view()
    }

    pub fn signed_in(&self) -> bool {
        self.account.is_some()
    }

    pub fn devices(&self) -> Option<DevicesView> {
        let device = self.account.as_ref()?;
        Some(devices_view(
            &self.devices,
            device.sync_status(),
            device.server_time(),
            &self.today.zone(),
        ))
    }

    pub fn sync_status(&self) -> Option<SyncStatus> {
        self.account.as_ref().map(Device::sync_status)
    }

    fn blocklist_result(&mut self, result: Result<Notice, Notice>, cx: &mut Context<Self>) -> bool {
        let ok = result.is_ok();
        self.blocklists_notice = Some(result.unwrap_or_else(|n| n));
        self.today.refresh(Utc::now());
        cx.notify();
        ok
    }

    pub fn add_entry(
        &mut self,
        list: &str,
        kind: EntryKind,
        input: &str,
        cx: &mut Context<Self>,
    ) -> bool {
        let result = self.blocklists.add(list, kind, input);
        self.blocklist_result(result, cx)
    }

    pub fn remove_entry(
        &mut self,
        list: &str,
        kind: EntryKind,
        entry: &str,
        cx: &mut Context<Self>,
    ) {
        let result = self.blocklists.remove(list, kind, entry);
        self.blocklist_result(result, cx);
    }

    pub fn set_category(&mut self, list: &str, on: bool, cx: &mut Context<Self>) {
        let result = if on {
            self.blocklists.opt_in(list)
        } else {
            self.blocklists.opt_out(list)
        };
        self.blocklist_result(result, cx);
    }

    pub fn start_quit_social(&mut self, cx: &mut Context<Self>) {
        let result = self.blocklists.start_quit_social();
        self.blocklist_result(result, cx);
    }
}

/// A macOS notification (warnings reach you even with the window closed).
fn notify(text: &str) {
    let script = format!(
        "display notification \"{}\" with title \"Cloister\"",
        text.replace('\\', "").replace('"', "'")
    );
    let _ = std::process::Command::new("/usr/bin/osascript")
        .args(["-e", &script])
        .spawn();
}
