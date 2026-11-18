//! The Shutdown Ritual's kiosk lock on macOS: AppKit presentation options (Dock and menu bar
//! hidden, process switching, Force Quit and Hide disabled) and window levels above everything
//! else on every Space. GPUI doesn't expose these, so they are set here through `objc2`.
//!
//! The options belong to this process: macOS drops them the moment it exits, so killing it
//! (from ssh, or `launchctl kill`) always returns the Mac to normal. While the ritual is due the
//! launch agent then starts it again within a second.
//!
//! Debug builds only: an escape hatch so development can't lock the Mac. Control-Option-
//! Command-Escape quits, and the lock lifts by itself after `CLOISTER_KIOSK_ESCAPE_SECS`
//! (default 120 s). `CLOISTER_KIOSK=off` shows the windows without the lock. Release builds
//! have none of this.

use std::time::Duration;

/// How the kiosk behaves in this build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Kiosk {
    /// Whether presentation options and window levels are applied.
    pub lock: bool,
    /// Debug builds: quit after this long.
    pub escape_after: Option<Duration>,
}

impl Kiosk {
    #[cfg(debug_assertions)]
    pub fn from_env() -> Self {
        let secs = std::env::var("CLOISTER_KIOSK_ESCAPE_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(120);
        Kiosk {
            lock: std::env::var("CLOISTER_KIOSK").as_deref() != Ok("off"),
            escape_after: Some(Duration::from_secs(secs)),
        }
    }

    #[cfg(not(debug_assertions))]
    pub fn from_env() -> Self {
        Kiosk {
            lock: true,
            escape_after: None,
        }
    }

    /// Debug builds: whether a key press is the escape chord (Control-Option-Command-Escape).
    pub fn is_escape(&self, keystroke: &gpui_kit::Keystroke) -> bool {
        cfg!(debug_assertions)
            && keystroke.key == "escape"
            && keystroke.modifiers.control
            && keystroke.modifiers.alt
            && keystroke.modifiers.platform
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{
        NSApplication, NSApplicationPresentationOptions as O, NSScreenSaverWindowLevel,
        NSWindowCollectionBehavior as B,
    };

    pub fn options() -> O {
        O::HideDock
            | O::HideMenuBar
            | O::DisableAppleMenu
            | O::DisableProcessSwitching
            | O::DisableForceQuit
            | O::DisableHideApplication
    }

    /// Lock: bring the app forward, set the options, and raise every window above the rest on
    /// every Space. Idempotent; called again whenever something might have undone it.
    pub fn lock() -> Option<O> {
        let mtm = MainThreadMarker::new()?;
        let app = NSApplication::sharedApplication(mtm);
        #[allow(deprecated)]
        app.activateIgnoringOtherApps(true);
        if app.presentationOptions() != options() {
            app.setPresentationOptions(options());
        }
        for window in app.windows().iter() {
            window.setLevel(NSScreenSaverWindowLevel);
            window.setCollectionBehavior(
                B::CanJoinAllSpaces | B::Stationary | B::FullScreenAuxiliary | B::IgnoresCycle,
            );
            // A normal window is kept below the menu bar; at this level it may cover the
            // whole screen, the strip beside a notch included.
            if let Some(screen) = window.screen() {
                if window.frame() != screen.frame() {
                    window.setFrame_display(screen.frame(), true);
                }
            }
            window.makeKeyAndOrderFront(None);
        }
        Some(app.currentSystemPresentationOptions())
    }

    pub fn unlock() {
        if let Some(mtm) = MainThreadMarker::new() {
            NSApplication::sharedApplication(mtm).setPresentationOptions(O::Default);
        }
    }

    pub fn describe(o: O) -> String {
        let names = [
            (O::HideDock, "HideDock"),
            (O::HideMenuBar, "HideMenuBar"),
            (O::DisableAppleMenu, "DisableAppleMenu"),
            (O::DisableProcessSwitching, "DisableProcessSwitching"),
            (O::DisableForceQuit, "DisableForceQuit"),
            (O::DisableHideApplication, "DisableHideApplication"),
        ];
        names
            .iter()
            .filter(|(flag, _)| o.contains(*flag))
            .map(|(_, name)| *name)
            .collect::<Vec<_>>()
            .join("|")
    }
}

/// Apply the lock. Returns the options now in effect for the system, in words (for the log).
pub fn lock() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        mac::lock().map(mac::describe)
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

/// Lift the lock (the process exiting does this too).
pub fn unlock() {
    #[cfg(target_os = "macos")]
    mac::unlock();
}
