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
