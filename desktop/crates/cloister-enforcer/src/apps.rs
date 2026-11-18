//! The app blocker's decisions: which running processes belong to a blocked app and must be
//! force-quit. Pure and unprivileged; the macOS layer only lists processes and kills pids.
//!
//! A blocked app entry (from the Enforcement) matches an app bundle when it equals, ignoring
//! case, the bundle's identifier (`com.hnc.Discord`) or its name (`Discord` for
//! `/Applications/Discord.app`). Every process inside the bundle (helpers included) is quit.
//! Apple's own apps and Cloister itself are never quit, whatever the Blocklists say.

use std::collections::BTreeSet;

/// A running process: its pid and the path of its executable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Process {
    pub pid: u32,
    pub executable: String,
}

/// Bundle identifiers that are never quit (prefixes).
const PROTECTED: &[&str] = &["com.apple.", "com.cloister."];

/// Parse `ps -axo pid=,comm=` output (`comm` is the full executable path on macOS).
pub fn parse_ps(output: &str) -> Vec<Process> {
    output
        .lines()
        .filter_map(|line| {
            let line = line.trim_start();
            let (pid, exe) = line.split_once(char::is_whitespace)?;
            Some(Process {
                pid: pid.parse().ok()?,
                executable: exe.trim().to_string(),
            })
        })
        .filter(|p| !p.executable.is_empty())
        .collect()
}

/// The outermost `.app` bundle containing `executable`: its path and its name.
pub fn outermost_app(executable: &str) -> Option<(String, String)> {
    let end = executable.find(".app/")?;
    let path = &executable[..end + 4];
    let name = path.rsplit('/').next()?.strip_suffix(".app")?;
    (!name.is_empty()).then(|| (path.to_string(), name.to_string()))
}

/// The pids to force-quit now. `bundle_id` looks up an app bundle's identifier by its path
/// (the macOS layer reads its Info.plist; tests pass a table).
pub fn processes_to_quit(
    processes: &[Process],
    blocked_apps: &[String],
    mut bundle_id: impl FnMut(&str) -> Option<String>,
) -> Vec<u32> {
    if blocked_apps.is_empty() {
        return Vec::new();
    }
    let blocked: BTreeSet<String> = blocked_apps.iter().map(|a| a.to_lowercase()).collect();
    let mut pids = Vec::new();
    for process in processes {
        let Some((path, name)) = outermost_app(&process.executable) else {
            continue;
        };
        let id = bundle_id(&path);
        if id
            .as_deref()
            .is_some_and(|id| PROTECTED.iter().any(|p| id.starts_with(p)))
        {
            continue;
        }
        let by_name = blocked.contains(&name.to_lowercase());
        let by_id = id.is_some_and(|id| blocked.contains(&id.to_lowercase()));
        if by_name || by_id {
            pids.push(process.pid);
        }
    }
    pids
}

/// Executable names (lower case, without `.exe`) that are never quit on Linux and Windows,
/// whatever the Blocklists say: the session itself and Cloister.
const PROTECTED_NAMES: &[&str] = &[
    // Linux
    "init",
    "systemd",
    "xorg",
    "xwayland",
    "gnome-shell",
    "kwin_wayland",
    "kwin_x11",
    "plasmashell",
    "sway",
    "mutter",
    "gdm",
    "sddm",
    "login",
    "dbus-daemon",
    // Windows
    "system",
    "system idle process",
    "smss",
    "csrss",
    "wininit",
    "winlogon",
    "services",
    "lsass",
    "svchost",
    "explorer",
    "dwm",
    "taskmgr",
    "conhost",
];

/// The name an executable path is matched by on Linux and Windows: its file name, without a
/// `.exe` extension, in lower case. Handles both `/` and `\` separators.
pub fn executable_name(executable: &str) -> String {
    let file = executable.rsplit(['/', '\\']).next().unwrap_or(executable);
    let lower = file.to_lowercase();
    lower.strip_suffix(".exe").unwrap_or(&lower).to_string()
}

/// The pids to force-quit on Linux and Windows, where apps have no bundle identifier. A blocked
/// entry matches an executable named like the entry or like the entry's last dot-separated
/// segment, ignoring case: `com.hnc.Discord` and `Discord` both match `Discord.exe` and
/// `/usr/share/discord/Discord`. The session and Cloister itself are never quit.
pub fn processes_to_quit_by_name(processes: &[Process], blocked_apps: &[String]) -> Vec<u32> {
    let mut names = BTreeSet::new();
    for entry in blocked_apps {
        let entry = entry.to_lowercase();
        if let Some((_, last)) = entry.rsplit_once('.') {
            if !last.is_empty() {
                names.insert(last.to_string());
            }
        }
        names.insert(entry);
    }
    processes
        .iter()
        .filter(|p| p.pid > 1)
        .filter(|p| {
            let name = executable_name(&p.executable);
            !PROTECTED_NAMES.contains(&name.as_str())
                && !name.starts_with("cloister")
                && names.contains(&name)
        })
        .map(|p| p.pid)
        .collect()
}

/// Parse `tasklist /fo csv /nh` output: image name and pid of every process.
pub fn parse_tasklist_csv(output: &str) -> Vec<Process> {
    output
        .lines()
        .filter_map(|line| {
            let mut fields = line.trim().split("\",\"");
            let image = fields.next()?.trim_start_matches('"');
            let pid = fields.next()?.parse().ok()?;
            Some(Process {
                pid,
                executable: image.to_string(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PS: &str = "    1 /sbin/launchd
  501 /Applications/Discord.app/Contents/MacOS/Discord
  502 /Applications/Discord.app/Contents/Frameworks/Discord Helper (Renderer).app/Contents/MacOS/Discord Helper (Renderer)
  610 /Applications/Safari.app/Contents/MacOS/Safari
  700 /Users/me/Applications/Steam.app/Contents/MacOS/steam_osx
  800 /Applications/Slack.app/Contents/MacOS/Slack
  900 /Applications/Cloister.app/Contents/MacOS/cloister-app
";

    fn ids(path: &str) -> Option<String> {
        match path {
            "/Applications/Discord.app" => Some("com.hnc.Discord".into()),
            "/Applications/Safari.app" => Some("com.apple.Safari".into()),
            "/Applications/Slack.app" => Some("com.tinyspeck.slackmacgap".into()),
            "/Applications/Cloister.app" => Some("com.cloister.app".into()),
            _ => None,
        }
    }

    fn blocked(apps: &[&str]) -> Vec<String> {
        apps.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_pids_and_executable_paths_with_spaces() {
        let ps = parse_ps(PS);
        assert_eq!(ps.len(), 7);
        assert_eq!(ps[2].pid, 502);
        assert!(ps[2].executable.ends_with("Discord Helper (Renderer)"));
    }

    #[test]
    fn a_blocked_bundle_id_quits_the_app_and_its_helpers() {
        let pids = processes_to_quit(&parse_ps(PS), &blocked(&["com.hnc.Discord"]), ids);
        assert_eq!(pids, [501, 502]);
    }

    #[test]
    fn an_app_name_matches_ignoring_case_even_without_a_bundle_id() {
        let pids = processes_to_quit(&parse_ps(PS), &blocked(&["steam", "SLACK"]), ids);
        assert_eq!(pids, [700, 800]);
    }

    #[test]
    fn apple_apps_and_cloister_are_never_quit() {
        let pids = processes_to_quit(
            &parse_ps(PS),
            &blocked(&["Safari", "com.apple.Safari", "Cloister", "com.cloister.app"]),
            ids,
        );
        assert!(pids.is_empty(), "{pids:?}");
    }

    #[test]
    fn nothing_is_quit_when_no_app_is_blocked() {
        assert!(processes_to_quit(&parse_ps(PS), &[], ids).is_empty());
        assert!(processes_to_quit(&parse_ps(PS), &blocked(&["Zoom"]), ids).is_empty());
    }
}

#[cfg(test)]
mod by_name_tests {
    use super::*;

    fn blocked(apps: &[&str]) -> Vec<String> {
        apps.iter().map(|s| s.to_string()).collect()
    }

    fn p(pid: u32, exe: &str) -> Process {
        Process {
            pid,
            executable: exe.into(),
        }
    }

    #[test]
    fn a_linux_executable_matches_an_entry_or_its_last_bundle_id_segment() {
        let procs = [
            p(10, "/usr/share/discord/Discord"),
            p(11, "/usr/lib/slack/slack"),
            p(12, "/usr/bin/steam"),
            p(13, "/usr/bin/firefox"),
        ];
        let pids =
            processes_to_quit_by_name(&procs, &blocked(&["com.hnc.Discord", "com.Slack", "Steam"]));
        assert_eq!(pids, [10, 11, 12]);
    }

    #[test]
    fn a_windows_image_name_matches_without_its_exe_extension() {
        let procs = [
            p(4, r"C:\Users\me\AppData\Local\Discord\app-1.0\Discord.exe"),
            p(5, "Teams.exe"),
            p(6, "notepad.exe"),
        ];
        let pids = processes_to_quit_by_name(&procs, &blocked(&["discord", "com.microsoft.teams"]));
        assert_eq!(pids, [4, 5]);
    }

    #[test]
    fn the_system_and_cloister_are_never_quit_by_name() {
        let procs = [
            p(1, "/sbin/init"),
            p(2, "/usr/bin/gnome-shell"),
            p(3, r"C:\Windows\explorer.exe"),
            p(7, "winlogon.exe"),
            p(8, "/usr/local/bin/cloister-app"),
            p(9, r"C:\Program Files\Cloister\cloisterd.exe"),
        ];
        let pids = processes_to_quit_by_name(
            &procs,
            &blocked(&[
                "init",
                "gnome-shell",
                "explorer",
                "winlogon",
                "cloister-app",
                "cloisterd",
            ]),
        );
        assert!(pids.is_empty(), "{pids:?}");
    }

    #[test]
    fn parses_proc_entries_and_tasklist_csv() {
        let csv = "\"System Idle Process\",\"0\",\"Services\",\"0\",\"8 K\"\r\n\"Discord.exe\",\"4242\",\"Console\",\"1\",\"120,000 K\"\r\n";
        assert_eq!(
            parse_tasklist_csv(csv),
            [p(0, "System Idle Process"), p(4242, "Discord.exe")]
        );
    }
}
