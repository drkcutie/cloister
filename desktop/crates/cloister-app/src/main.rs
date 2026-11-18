//! `cloister-app`: the Cloister desktop app (GPUI).
//!
//! It starts Blocks only by asking the enforcement service over IPC (`$CLOISTER_SOCKET`, or
//! `/var/run/cloister.sock`), so closing or quitting it never affects a Block, and reopening
//! it shows whatever is running. Everything it shows comes from `cloister-app-model`.

mod screens;
mod state;
mod theme;
mod tray;
mod ui;

use gpui_kit::*;

use crate::state::{Cloister, CloisterHandle};

fn main() {
    let app = gpui_kit::application();
    // Clicking the Dock icon with no window open brings the main window back.
    app.on_reopen(screens::open_main_window);
    app.run(|cx| {
        gpui_kit::init(cx);
        theme::load_fonts(cx);
        theme::install(cx);
        let cloister = cx.new(Cloister::new);
        cx.set_global(CloisterHandle(cloister));
        tray::install(cx);
        screens::open_main_window(cx);
    });
}
