//! The menu-bar item: the arch, the time left in a Deep Session as its title, and a menu to
//! open the main window. GPUI has no status-item API, so this uses `tray-icon`.

use std::time::Duration;

use cloister_app_model::format;
use gpui_kit::*;
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{TrayIcon, TrayIconBuilder};

use crate::screens::open_main_window;
use crate::state::CloisterHandle;

struct Tray {
    icon: TrayIcon,
    status: MenuItem,
    open_id: tray_icon::menu::MenuId,
    quit_id: tray_icon::menu::MenuId,
    last_title: Option<String>,
}

/// Create the menu-bar item. Must run on the main thread inside `Application::run`.
pub fn install(cx: &mut App) {
    let status = MenuItem::new("No Block in force", false, None);
    let open = MenuItem::new("Open Cloister", true, None);
    // Quitting the app never touches a Block: the enforcement service keeps it in force.
    let quit = MenuItem::new("Quit Cloister", true, None);
    let menu = Menu::new();
    let built = menu
        .append_items(&[&status, &PredefinedMenuItem::separator(), &open, &quit])
        .map_err(|e| e.to_string())
        .and_then(|_| {
            TrayIconBuilder::new()
                .with_tooltip("Cloister")
                .with_icon_templated(arch_icon())
                .with_menu(Box::new(menu))
                .build()
                .map_err(|e| e.to_string())
        });
    let icon = match built {
        Ok(icon) => icon,
        Err(e) => {
            eprintln!("cloister-app: menu-bar item unavailable: {e}");
            return;
        }
    };
    let mut tray = Tray {
        icon,
        status,
        open_id: open.id().clone(),
        quit_id: quit.id().clone(),
        last_title: None,
    };
    cx.spawn(async move |cx| loop {
        cx.background_executor()
            .timer(Duration::from_millis(200))
            .await;
        cx.update(|cx| tray.tick(cx));
    })
    .detach();
}

impl Tray {
    fn tick(&mut self, cx: &mut App) {
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            if event.id == self.open_id {
                open_main_window(cx);
            } else if event.id == self.quit_id {
                cx.quit();
            }
        }
        let Some(handle) = cx.try_global::<CloisterHandle>() else {
            return;
        };
        let cloister = handle.0.read(cx);
        let view = cloister.view();
        let zone = cloister.today().zone();
        let (title, status) = match &view.active {
            Some(a) => {
                let mins = (a.remaining.num_seconds() + 59) / 60;
                let left = format::hours(chrono::Duration::minutes(mins));
                let kind = if a.deep { "Deep Session" } else { "Block" };
                (
                    Some(left),
                    format!("{kind} ends at {}", format::clock(a.ends_at, &zone)),
                )
            }
            None if !view.reachable => (None, "Enforcement service not running".into()),
            None => (None, "No Block in force".into()),
        };
        if title != self.last_title {
            self.icon.set_title(title.as_deref());
            let _ = self.icon.set_tooltip(Some(format!("Cloister: {status}")));
            self.status.set_text(&status);
            self.last_title = title;
        } else {
            self.status.set_text(&status);
        }
    }
}

/// The arch as a 36×36 template image (drawn, so it stays crisp at 18pt @2x). The full
/// "archway of light" logo is too detailed for the menu bar.
fn arch_icon() -> tray_icon::Icon {
    const N: usize = 36;
    const SS: usize = 4; // supersampling
    let inside = |x: f32, y: f32, half: f32, top: f32, bottom: f32| -> bool {
        let cx = N as f32 / 2.;
        if y > bottom {
            false
        } else if y >= top {
            (x - cx).abs() <= half
        } else {
            ((x - cx).powi(2) + (y - top).powi(2)).sqrt() <= half
        }
    };
    let mut rgba = vec![0u8; N * N * 4];
    for py in 0..N {
        for px in 0..N {
            let mut hits = 0;
            for sy in 0..SS {
                for sx in 0..SS {
                    let x = px as f32 + (sx as f32 + 0.5) / SS as f32;
                    let y = py as f32 + (sy as f32 + 0.5) / SS as f32;
                    let outer = inside(x, y, 13., 15., 33.);
                    let hollow = inside(x, y, 9.5, 15., 30.);
                    let door = inside(x, y, 3.5, 18., 30.);
                    if (outer && !hollow) || door {
                        hits += 1;
                    }
                }
            }
            let i = (py * N + px) * 4;
            rgba[i + 3] = (hits * 255 / (SS * SS)) as u8;
        }
    }
    tray_icon::Icon::from_rgba(rgba, N as u32, N as u32).expect("valid icon size")
}
