//! Installing the enforcement service, on every desktop OS, as a plan of steps: copy the
//! binary, write the service definitions, register and start them. The plan is pure (tested
//! here); `cloisterd install` runs it as root / Administrator, and the app asks for that with one
//! administrator prompt ([`elevated_install`]).

use std::path::Path;

/// The desktop operating systems Cloister enforces on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    MacOs,
    Linux,
    Windows,
}

impl Os {
    pub fn current() -> Self {
        if cfg!(target_os = "windows") {
            Os::Windows
        } else if cfg!(target_os = "linux") {
            Os::Linux
        } else {
            Os::MacOs
        }
    }
}

/// One step of an install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Copy a file (skipped when source and destination are the same file).
    Copy { from: String, to: String, mode: u32 },
    Write {
        path: String,
        contents: String,
        mode: u32,
    },
    /// Run a program. `may_fail`: an error is expected when there is nothing to undo first.
    Run {
        program: String,
        args: Vec<String>,
        may_fail: bool,
    },
}

/// Where the service and the app live after installing, per OS.
pub struct Paths;

impl Paths {
    pub fn daemon(os: Os) -> &'static str {
        match os {
            Os::MacOs | Os::Linux => "/usr/local/libexec/cloisterd",
            Os::Windows => r"C:\Program Files\Cloister\cloisterd.exe",
        }
    }
}

fn run(program: &str, args: &[&str], may_fail: bool) -> Step {
    Step::Run {
        program: program.into(),
        args: args.iter().map(|a| a.to_string()).collect(),
        may_fail,
    }
}

fn write(path: &str, contents: String) -> Step {
    Step::Write {
        path: path.into(),
        contents,
        mode: 0o644,
    }
}

/// The steps that install the service from the binary at `daemon` (copied into place), with
/// the ritual window shown by the app at `app`.
pub fn plan(os: Os, daemon: &str, app: &str) -> Vec<Step> {
    let installed = Paths::daemon(os);
    let mut steps = vec![Step::Copy {
        from: daemon.into(),
        to: installed.into(),
        mode: 0o755,
    }];
    match os {
        Os::MacOs => {
            use crate::macos::{launchd_plist, ritual_agent_plist, LABEL, PLIST_PATH};
            use crate::macos::{RITUAL_FLAG_PATH, RITUAL_PLIST_PATH};
            steps.push(write(PLIST_PATH, launchd_plist(installed)));
            steps.push(write(
                RITUAL_PLIST_PATH,
                ritual_agent_plist(app, RITUAL_FLAG_PATH),
            ));
            steps.push(run(
                "/bin/launchctl",
                &["bootout", "system", PLIST_PATH],
                true,
            ));
            steps.push(run(
                "/bin/launchctl",
                &["bootstrap", "system", PLIST_PATH],
                false,
            ));
            steps.push(run(
                "/bin/launchctl",
                &["enable", &format!("system/{LABEL}")],
                false,
            ));
        }
        Os::Linux => {
            use crate::linux::*;
            steps.push(write(UNIT_PATH, service_unit(installed)));
            steps.push(write(RITUAL_UNIT_PATH, ritual_service_unit(app)));
            steps.push(write(RITUAL_PATH_UNIT_PATH, ritual_path_unit()));
            steps.push(run("systemctl", &["daemon-reload"], false));
            steps.push(run("systemctl", &["enable", "--now", UNIT], false));
            // A reinstall replaces the binary: restart picks it up.
            steps.push(run("systemctl", &["restart", UNIT], false));
            steps.push(run(
                "systemctl",
                &["--global", "enable", RITUAL_PATH_UNIT],
                false,
            ));
        }
        Os::Windows => {
            use crate::windows::SERVICE_NAME;
            let bin_path = format!("\"{installed}\" service --app \"{app}\"");
            steps.push(run("sc.exe", &["stop", SERVICE_NAME], true));
            steps.push(run(
                "sc.exe",
                &[
                    "create",
                    SERVICE_NAME,
                    "binPath=",
                    &bin_path,
                    "start=",
                    "auto",
                    "DisplayName=",
                    "Cloister",
                ],
                true,
            ));
            steps.push(run(
                "sc.exe",
                &[
                    "config",
                    SERVICE_NAME,
                    "binPath=",
                    &bin_path,
                    "start=",
                    "auto",
                ],
                false,
            ));
            steps.push(run(
                "sc.exe",
                &[
                    "failure",
                    SERVICE_NAME,
                    "reset=",
                    "0",
                    "actions=",
                    "restart/1000/restart/1000/restart/1000",
                ],
                false,
            ));
            steps.push(run("sc.exe", &["start", SERVICE_NAME], false));
        }
    }
    steps
}

/// The one command the app runs to install the service: it shows the OS's own administrator
/// prompt once and then runs `cloisterd install --app APP` with full rights.
pub fn elevated_install(os: Os, daemon: &str, app: &str) -> (String, Vec<String>) {
    match os {
        Os::MacOs => {
            let script = format!("{} install --app {}", sh_quote(daemon), sh_quote(app));
            let applescript = format!(
                "do shell script \"{}\" with administrator privileges",
                script.replace('\\', "\\\\").replace('"', "\\\"")
            );
            ("/usr/bin/osascript".into(), vec!["-e".into(), applescript])
        }
        Os::Linux => (
            "pkexec".into(),
            vec![daemon.into(), "install".into(), "--app".into(), app.into()],
        ),
        Os::Windows => {
            let ps = |s: &str| format!("'{}'", s.replace('\'', "''"));
            (
                "powershell.exe".into(),
                vec![
                    "-NoProfile".into(),
                    "-Command".into(),
                    format!(
                        "Start-Process -Verb RunAs -Wait -FilePath {} -ArgumentList 'install','--app',{}",
                        ps(daemon),
                        ps(app)
                    ),
                ],
            )
        }
    }
}

/// Quote for `sh`: wrap in single quotes; a single quote becomes `'\''`.
fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Run `steps` (as root / Administrator). Stops at the first step that fails unless it may.
pub fn execute(steps: &[Step]) -> Result<(), String> {
    for step in steps {
        match step {
            Step::Copy { from, to, mode } => {
                if same_file(from, to) {
                    continue;
                }
                if let Some(dir) = Path::new(to).parent() {
                    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
                }
                // Copy beside it and rename, so a running binary is replaced, not truncated.
                let tmp = format!("{to}.new");
                std::fs::copy(from, &tmp).map_err(|e| format!("copying {from}: {e}"))?;
                set_mode(&tmp, *mode)?;
                std::fs::rename(&tmp, to).map_err(|e| format!("{to}: {e}"))?;
            }
            Step::Write {
                path,
                contents,
                mode,
            } => {
                if let Some(dir) = Path::new(path).parent() {
                    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
                }
                std::fs::write(path, contents).map_err(|e| format!("{path}: {e}"))?;
                set_mode(path, *mode)?;
            }
            Step::Run {
                program,
                args,
                may_fail,
            } => {
                let status = std::process::Command::new(program).args(args).status();
                let ok = matches!(&status, Ok(s) if s.success());
                if !ok && !may_fail {
                    return Err(format!("{program} {}: {status:?}", args.join(" ")));
                }
            }
        }
    }
    Ok(())
}

fn same_file(a: &str, b: &str) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

#[cfg(unix)]
fn set_mode(path: &str, mode: u32) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .map_err(|e| format!("{path}: {e}"))
}

#[cfg(not(unix))]
fn set_mode(_: &str, _: u32) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runs(plan: &[Step]) -> Vec<String> {
        plan.iter()
            .filter_map(|s| match s {
                Step::Run { program, args, .. } => Some(format!("{program} {}", args.join(" "))),
                _ => None,
            })
            .collect()
    }

    fn written<'a>(plan: &'a [Step], path: &str) -> &'a str {
        plan.iter()
            .find_map(|s| match s {
                Step::Write {
                    path: p, contents, ..
                } if p == path => Some(contents.as_str()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("{path} not written: {plan:#?}"))
    }

    #[test]
    fn macos_installs_a_launch_daemon_and_the_ritual_agent() {
        let plan = plan(
            Os::MacOs,
            "/tmp/build/cloisterd",
            "/Applications/Cloister.app/Contents/MacOS/cloister-app",
        );
        assert_eq!(
            plan[0],
            Step::Copy {
                from: "/tmp/build/cloisterd".into(),
                to: "/usr/local/libexec/cloisterd".into(),
                mode: 0o755
            }
        );
        assert!(
            written(&plan, "/Library/LaunchDaemons/com.cloister.enforcer.plist")
                .contains("<string>/usr/local/libexec/cloisterd</string>")
        );
        assert!(
            written(&plan, "/Library/LaunchAgents/com.cloister.ritual.plist").contains(
                "<string>/Applications/Cloister.app/Contents/MacOS/cloister-app</string>"
            )
        );
        assert_eq!(
            runs(&plan),
            [
                "/bin/launchctl bootout system /Library/LaunchDaemons/com.cloister.enforcer.plist",
                "/bin/launchctl bootstrap system /Library/LaunchDaemons/com.cloister.enforcer.plist",
                "/bin/launchctl enable system/com.cloister.enforcer",
            ]
        );
    }

    #[test]
    fn linux_installs_a_systemd_service_and_a_ritual_unit_for_every_user() {
        let plan = plan(Os::Linux, "/home/me/cloisterd", "/usr/bin/cloister-app");
        let unit = written(&plan, "/etc/systemd/system/cloisterd.service");
        assert!(unit.contains("ExecStart=/usr/local/libexec/cloisterd run\n"));
        assert!(unit.contains("Restart=always\n"));
        assert!(unit.contains("WantedBy=multi-user.target\n"));
        let ritual = written(&plan, "/etc/systemd/user/cloister-ritual.service");
        assert!(ritual.contains("ExecStart=/usr/bin/cloister-app --ritual\n"));
        assert!(ritual.contains("ConditionPathExists=/run/cloister-ritual-due\n"));
        assert!(ritual.contains("Restart=always\nRestartSec=1\n"));
        assert!(
            ritual.contains("StartLimitIntervalSec=0\n"),
            "never stops relaunching"
        );
        let path = written(&plan, "/etc/systemd/user/cloister-ritual.path");
        assert!(path.contains("PathExists=/run/cloister-ritual-due\n"));
        assert_eq!(
            runs(&plan),
            [
                "systemctl daemon-reload",
                "systemctl enable --now cloisterd.service",
                "systemctl restart cloisterd.service",
                "systemctl --global enable cloister-ritual.path",
            ]
        );
    }

    #[test]
    fn windows_registers_a_service_that_restarts_and_knows_the_app() {
        let plan = plan(
            Os::Windows,
            r"C:\Users\me\Downloads\cloisterd.exe",
            r"C:\Program Files\Cloister\cloister-app.exe",
        );
        assert_eq!(
            plan[0],
            Step::Copy {
                from: r"C:\Users\me\Downloads\cloisterd.exe".into(),
                to: r"C:\Program Files\Cloister\cloisterd.exe".into(),
                mode: 0o755
            }
        );
        let runs = runs(&plan);
        assert_eq!(runs[0], "sc.exe stop Cloister");
        assert_eq!(
            runs[1],
            r#"sc.exe create Cloister binPath= "C:\Program Files\Cloister\cloisterd.exe" service --app "C:\Program Files\Cloister\cloister-app.exe" start= auto DisplayName= Cloister"#
        );
        assert_eq!(
            runs[2],
            r#"sc.exe config Cloister binPath= "C:\Program Files\Cloister\cloisterd.exe" service --app "C:\Program Files\Cloister\cloister-app.exe" start= auto"#
        );
        assert_eq!(
            runs[3],
            "sc.exe failure Cloister reset= 0 actions= restart/1000/restart/1000/restart/1000"
        );
        assert_eq!(runs[4], "sc.exe start Cloister");
    }

    #[test]
    fn macos_asks_for_the_admin_password_once_through_osascript() {
        let (program, args) = elevated_install(
            Os::MacOs,
            "/Applications/Cloister.app/Contents/MacOS/cloisterd",
            "/Applications/Cloister.app/Contents/MacOS/cloister-app",
        );
        assert_eq!(program, "/usr/bin/osascript");
        assert_eq!(
            args,
            [
                "-e",
                r#"do shell script "'/Applications/Cloister.app/Contents/MacOS/cloisterd' install --app '/Applications/Cloister.app/Contents/MacOS/cloister-app'" with administrator privileges"#,
            ]
        );
    }

    #[test]
    fn quotes_in_paths_cant_break_out_of_the_admin_shell_script() {
        let (_, args) = elevated_install(
            Os::MacOs,
            "/Users/o'neil/bin/cloisterd",
            "/Apps/a\"b/cloister-app",
        );
        // ' is closed, escaped and reopened for sh; " and \ are escaped for AppleScript.
        assert_eq!(
            args[1],
            r#"do shell script "'/Users/o'\\''neil/bin/cloisterd' install --app '/Apps/a\"b/cloister-app'" with administrator privileges"#
        );
    }

    #[test]
    fn linux_and_windows_use_their_own_admin_prompts() {
        let (program, args) = elevated_install(
            Os::Linux,
            "/opt/cloister/cloisterd",
            "/opt/cloister/cloister-app",
        );
        assert_eq!(program, "pkexec");
        assert_eq!(
            args,
            [
                "/opt/cloister/cloisterd",
                "install",
                "--app",
                "/opt/cloister/cloister-app"
            ]
        );
        let (program, args) = elevated_install(
            Os::Windows,
            r"C:\Cloister\cloisterd.exe",
            r"C:\Cloister\cloister-app.exe",
        );
        assert_eq!(program, "powershell.exe");
        assert_eq!(
            args,
            [
                "-NoProfile",
                "-Command",
                r"Start-Process -Verb RunAs -Wait -FilePath 'C:\Cloister\cloisterd.exe' -ArgumentList 'install','--app','C:\Cloister\cloister-app.exe'",
            ]
        );
    }
}
