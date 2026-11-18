//! System notifications on every desktop OS (the Shutdown Time warnings reach you with the
//! window closed): the command that shows one, built here so it is tested everywhere.

/// The desktop operating systems.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesktopOs {
    MacOs,
    Linux,
    Windows,
}

impl DesktopOs {
    pub fn current() -> Self {
        if cfg!(target_os = "windows") {
            DesktopOs::Windows
        } else if cfg!(target_os = "linux") {
            DesktopOs::Linux
        } else {
            DesktopOs::MacOs
        }
    }
}

/// The program and arguments that show `text` as a notification titled "Cloister" on `os`.
/// Quotes in the text can't break out of the script.
pub fn notification_command(os: DesktopOs, text: &str) -> (String, Vec<String>) {
    match os {
        DesktopOs::MacOs => {
            let text = text.replace(['\\', '"'], "'");
            (
                "/usr/bin/osascript".into(),
                vec![
                    "-e".into(),
                    format!("display notification \"{text}\" with title \"Cloister\""),
                ],
            )
        }
        DesktopOs::Linux => (
            "notify-send".into(),
            vec!["--app-name=Cloister".into(), "Cloister".into(), text.into()],
        ),
        DesktopOs::Windows => {
            // A toast through WinRT, shown as Windows PowerShell (an app id Windows knows).
            let text = text.replace('\'', "''");
            let script = format!(
                "[Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime] > $null; \
                 $x = [Windows.UI.Notifications.ToastNotificationManager]::GetTemplateContent([Windows.UI.Notifications.ToastTemplateType]::ToastText02); \
                 $t = $x.GetElementsByTagName('text'); \
                 $t.Item(0).AppendChild($x.CreateTextNode('Cloister')) > $null; \
                 $t.Item(1).AppendChild($x.CreateTextNode('{text}')) > $null; \
                 [Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('{{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}}\\WindowsPowerShell\\v1.0\\powershell.exe').Show([Windows.UI.Notifications.ToastNotification]::new($x))"
            );
            (
                "powershell.exe".into(),
                vec![
                    "-NoProfile".into(),
                    "-WindowStyle".into(),
                    "Hidden".into(),
                    "-Command".into(),
                    script,
                ],
            )
        }
    }
}

/// Show `text` as a system notification (best effort; never blocks).
pub fn notify(text: &str) {
    let (program, args) = notification_command(DesktopOs::current(), text);
    let _ = std::process::Command::new(program).args(args).spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_desktop_gets_a_notification_with_the_text() {
        let text = "Shutdown Time in 15 minutes, at 17:00.";
        for os in [DesktopOs::MacOs, DesktopOs::Linux, DesktopOs::Windows] {
            let (_, args) = notification_command(os, text);
            assert!(args.iter().any(|a| a.contains(text)), "{os:?}");
        }
    }

    #[test]
    fn quotes_cant_break_out_of_the_script() {
        let (_, mac) = notification_command(DesktopOs::MacOs, r#"a" & do shell script "x"#);
        assert!(!mac[1].contains(r#"a" &"#));
        let (_, win) = notification_command(DesktopOs::Windows, "it's'; Remove-Item x; '");
        assert!(win[4].contains("it''s''; Remove-Item x; ''"));
    }
}
