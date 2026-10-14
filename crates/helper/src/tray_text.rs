//! What the tray icon shows, as pure functions: texts in English and French, the
//! helper's state as the user sees it, the link that opens Faulk and pairs it, and the
//! command line stored for "start with Windows".
//!
//! Kept free of any platform API so it compiles and is tested everywhere.

use crate::updater::UpdateStatus;

/// Origin of the page a release build opens: the production site.
pub const RELEASE_APP_ORIGIN: &str = "https://www.faulk.app";
/// Origin of the page a debug build opens: the web app's dev server (`pnpm dev`).
pub const DEV_APP_ORIGIN: &str = "http://localhost:5173";
/// Path of the DAW on that origin: the landing page takes `/`, the DAW lives at `/app`.
pub const APP_PATH: &str = "/app";

/// The DAW's address for a release build (`release`) or a debug build.
pub fn app_url(release: bool) -> String {
    let origin = if release {
        RELEASE_APP_ORIGIN
    } else {
        DEV_APP_ORIGIN
    };
    format!("{origin}{APP_PATH}")
}

/// The page the tray opens unless `--app-url` says otherwise: the production site in a
/// release build (what the installer ships), the dev server in a debug build.
pub fn default_app_url() -> String {
    app_url(!cfg!(debug_assertions))
}

/// Language of the tray texts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    En,
    Fr,
}

/// Picks the language from a Windows LANGID (`GetUserDefaultUILanguage`): French for
/// every French locale (primary language 0x0C), English otherwise.
pub fn lang_from_langid(langid: u16) -> Lang {
    if langid & 0x3FF == 0x0C {
        Lang::Fr
    } else {
        Lang::En
    }
}

/// The state shown in the menu and the tooltip.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrayState {
    /// Current pairing code, 8 digits.
    pub code: String,
    pub page_connected: bool,
    /// The audio output reported an error; it stays until the helper restarts.
    pub audio_error: bool,
}

/// Menu entries that never change with the state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Label {
    OpenFaulk,
    StartAtLogin,
    CheckForUpdates,
    Quit,
}

/// Formats a pairing code for reading: `1234 5678`.
pub fn spaced_code(code: &str) -> String {
    if code.len() == 8 && code.is_ascii() {
        format!("{} {}", &code[..4], &code[4..])
    } else {
        code.to_owned()
    }
}

impl TrayState {
    pub fn status(&self, lang: Lang) -> &'static str {
        match (lang, self.audio_error, self.page_connected) {
            (Lang::En, true, _) => "Audio error: restart the helper",
            (Lang::Fr, true, _) => "Erreur audio : relance le helper",
            (Lang::En, false, true) => "Page connected",
            (Lang::Fr, false, true) => "Page connectée",
            (Lang::En, false, false) => "Waiting for a page",
            (Lang::Fr, false, false) => "En attente d’une page",
        }
    }

    pub fn tooltip(&self, lang: Lang) -> String {
        match lang {
            Lang::En => format!("Faulk helper: {}", self.status(lang)),
            Lang::Fr => format!("Helper Faulk : {}", self.status(lang)),
        }
    }

    /// The menu entry that shows the code; clicking it copies the code.
    pub fn code_entry(&self, lang: Lang) -> String {
        let code = spaced_code(&self.code);
        match lang {
            Lang::En => format!("Pairing code: {code} (click to copy)"),
            Lang::Fr => format!("Code d’appairage : {code} (clic pour copier)"),
        }
    }
}

pub fn label(lang: Lang, label: Label) -> &'static str {
    match (lang, label) {
        (Lang::En, Label::OpenFaulk) => "Open Faulk",
        (Lang::Fr, Label::OpenFaulk) => "Ouvrir Faulk",
        (Lang::En, Label::StartAtLogin) => "Start with Windows",
        (Lang::Fr, Label::StartAtLogin) => "Lancer au démarrage de Windows",
        (Lang::En, Label::CheckForUpdates) => "Check for updates",
        (Lang::Fr, Label::CheckForUpdates) => "Rechercher des mises à jour",
        (Lang::En, Label::Quit) => "Quit",
        (Lang::Fr, Label::Quit) => "Quitter",
    }
}

/// The menu entry shown once an update is downloaded: clicking it applies the update and
/// restarts the helper on `version`.
pub fn restart_to_update(lang: Lang, version: &str) -> String {
    match lang {
        Lang::En => format!("Restart to update to {version}"),
        Lang::Fr => format!("Redémarrer pour passer à la {version}"),
    }
}

/// What the menu must do with the "restart to update" entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EntryChange {
    /// Nothing changes.
    Keep,
    /// Insert the entry with this text.
    Show(String),
    /// The entry is shown; replace its text (a newer version was downloaded).
    Retext(String),
    /// Remove the entry.
    Hide,
}

/// The "restart to update" entry: shown while an update is ready, with its version. A
/// check or a download keeps it as it is: the package already downloaded stays applicable
/// meanwhile (the updater goes back to `Ready` after a check that finds nothing newer), and
/// hiding it at every periodic check would make it flicker.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RestartEntry {
    shown: Option<String>,
}

impl RestartEntry {
    /// Follows the updater's status.
    pub fn follow_status(&mut self, lang: Lang, status: &UpdateStatus) -> EntryChange {
        match status {
            UpdateStatus::Ready { version } => self.follow(lang, Some(version)),
            UpdateStatus::Checking | UpdateStatus::Downloading { .. } => EntryChange::Keep,
            UpdateStatus::Disabled { .. }
            | UpdateStatus::Idle
            | UpdateStatus::UpToDate { .. }
            | UpdateStatus::Failed { .. } => self.follow(lang, None),
        }
    }

    /// Follows a ready version: `ready` is the version of the downloaded update, if any.
    pub fn follow(&mut self, lang: Lang, ready: Option<&str>) -> EntryChange {
        let wanted = ready.map(|version| restart_to_update(lang, version));
        if wanted == self.shown {
            return EntryChange::Keep;
        }
        let change = match (&self.shown, &wanted) {
            (None, Some(text)) => EntryChange::Show(text.clone()),
            (Some(_), Some(text)) => EntryChange::Retext(text.clone()),
            (Some(_), None) => EntryChange::Hide,
            (None, None) => EntryChange::Keep,
        };
        self.shown = wanted;
        change
    }

    /// Whether the shown entry can be clicked: while the updater checks or downloads, an apply
    /// is refused (busy), so the entry stays, greyed, rather than answer a click with nothing.
    pub fn clickable(status: &UpdateStatus) -> bool {
        !matches!(
            status,
            UpdateStatus::Checking | UpdateStatus::Downloading { .. }
        )
    }

    pub fn is_shown(&self) -> bool {
        self.shown.is_some()
    }
}

/// First line of the message shown when the helper cannot start.
pub fn startup_failed(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "The Faulk helper could not start.",
        Lang::Fr => "Le helper Faulk n’a pas pu démarrer.",
    }
}

/// The page link that pairs by itself: the code goes in the fragment, which the
/// browser never sends to a server, and never in the query.
pub fn pairing_url(app_url: &str, code: &str) -> String {
    let base = app_url.split('#').next().unwrap_or(app_url);
    format!("{base}#pair={code}")
}

/// The command line stored in the Run key: the executable, quoted, then the arguments
/// the helper was started with, each quoted as Windows splits a command line.
pub fn run_command(exe: &str, args: &[String]) -> String {
    let mut line = format!("\"{exe}\"");
    for arg in args {
        line.push(' ');
        line.push_str(&quote_arg(arg));
    }
    line
}

/// Whether a command stored in the Run key starts `exe` (ASCII letters compared without
/// case; both sides come from `current_exe()`, so other letters match as written). The uninstaller removes only such a value: one left by
/// another copy of the helper, a development build for instance, is not its to remove.
pub fn run_command_starts(stored: &str, exe: &str) -> bool {
    let quoted = format!("\"{exe}\"");
    match stored.get(..quoted.len()) {
        Some(head) if head.eq_ignore_ascii_case(&quoted) => {
            matches!(stored[quoted.len()..].chars().next(), None | Some(' '))
        }
        _ => false,
    }
}

/// Quotes one argument the way Windows programs split their command line
/// (`CommandLineToArgvW`, the C runtime): backslashes are literal except before a
/// quote, where they are doubled, and quotes are escaped.
fn quote_arg(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '"']) {
        return arg.to_owned();
    }
    let mut out = String::from("\"");
    let mut backslashes = 0;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                out.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            _ => {
                out.extend(std::iter::repeat_n('\\', backslashes));
                out.push(c);
                backslashes = 0;
            }
        }
    }
    out.extend(std::iter::repeat_n('\\', backslashes * 2));
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::updater::UpdateError;

    const LANGS: [Lang; 2] = [Lang::En, Lang::Fr];

    #[test]
    fn a_run_command_starts_its_own_executable_only() {
        let exe = r"C:\Users\u\AppData\Local\FaulkHelper\current\faulk-helper.exe";
        let own = run_command(exe, &["--port".into(), "47801".into()]);
        assert!(run_command_starts(&own, exe));
        assert!(
            run_command_starts(&format!("\"{exe}\""), exe),
            "no arguments"
        );
        assert!(
            run_command_starts(&own.to_uppercase(), exe),
            "paths compare without case"
        );
        assert!(!run_command_starts(
            r#""C:\git\music\target\release\faulk-helper.exe" --port 47801"#,
            exe
        ));
        assert!(
            !run_command_starts(&format!("\"{exe}.old\""), exe),
            "a longer path is another file"
        );
        assert!(!run_command_starts(&format!("\"{exe}\"x"), exe));
        assert!(!run_command_starts("", exe));
        assert!(!run_command_starts("\"C:\\é", exe), "short, non-ASCII");
    }

    #[test]
    fn french_for_every_french_locale_english_otherwise() {
        assert_eq!(lang_from_langid(0x040C), Lang::Fr); // fr-FR
        assert_eq!(lang_from_langid(0x080C), Lang::Fr); // fr-BE
        assert_eq!(lang_from_langid(0x0C0C), Lang::Fr); // fr-CA
        assert_eq!(lang_from_langid(0x100C), Lang::Fr); // fr-CH
        assert_eq!(lang_from_langid(0x0409), Lang::En); // en-US
        assert_eq!(lang_from_langid(0x0407), Lang::En); // de-DE
        assert_eq!(lang_from_langid(0x0413), Lang::En); // nl-NL
        assert_eq!(lang_from_langid(0), Lang::En);
    }

    #[test]
    fn status_follows_the_state_and_audio_error_wins() {
        let mut state = TrayState {
            code: "12345678".into(),
            ..TrayState::default()
        };
        assert_eq!(state.status(Lang::En), "Waiting for a page");
        state.page_connected = true;
        assert_eq!(state.status(Lang::En), "Page connected");
        state.audio_error = true;
        assert_eq!(state.status(Lang::En), "Audio error: restart the helper");
        assert_eq!(state.status(Lang::Fr), "Erreur audio : relance le helper");
        state.page_connected = false;
        assert_eq!(state.status(Lang::En), "Audio error: restart the helper");
    }

    #[test]
    fn every_text_exists_in_both_languages_and_differs() {
        let states = [
            TrayState::default(),
            TrayState {
                page_connected: true,
                ..TrayState::default()
            },
            TrayState {
                audio_error: true,
                ..TrayState::default()
            },
        ];
        for state in &states {
            assert_ne!(state.status(Lang::En), state.status(Lang::Fr));
            assert_ne!(state.tooltip(Lang::En), state.tooltip(Lang::Fr));
            assert_ne!(state.code_entry(Lang::En), state.code_entry(Lang::Fr));
        }
        for l in [
            Label::OpenFaulk,
            Label::StartAtLogin,
            Label::CheckForUpdates,
            Label::Quit,
        ] {
            for lang in LANGS {
                assert!(!label(lang, l).is_empty());
            }
            assert_ne!(label(Lang::En, l), label(Lang::Fr, l));
        }
        assert_ne!(startup_failed(Lang::En), startup_failed(Lang::Fr));
        assert_ne!(
            restart_to_update(Lang::En, "0.2.0"),
            restart_to_update(Lang::Fr, "0.2.0")
        );
    }

    #[test]
    fn the_restart_entry_follows_the_ready_version() {
        let mut entry = RestartEntry::default();
        assert!(!entry.is_shown());
        assert_eq!(
            entry.follow(Lang::En, None),
            EntryChange::Keep,
            "never shown"
        );
        assert_eq!(
            entry.follow(Lang::En, Some("0.2.0")),
            EntryChange::Show("Restart to update to 0.2.0".into())
        );
        assert!(entry.is_shown());
        assert_eq!(entry.follow(Lang::En, Some("0.2.0")), EntryChange::Keep);
        assert_eq!(
            entry.follow(Lang::En, Some("0.2.1")),
            EntryChange::Retext("Restart to update to 0.2.1".into())
        );
        assert_eq!(entry.follow(Lang::En, None), EntryChange::Hide);
        assert!(!entry.is_shown());
        assert_eq!(entry.follow(Lang::En, None), EntryChange::Keep);
        assert_eq!(
            entry.follow(Lang::Fr, Some("0.3.0-beta.1")),
            EntryChange::Show("Redémarrer pour passer à la 0.3.0-beta.1".into())
        );
    }

    #[test]
    fn the_restart_entry_follows_every_update_status() {
        let ready = |v: &str| UpdateStatus::Ready { version: v.into() };
        let mut entry = RestartEntry::default();
        for hidden in [
            UpdateStatus::Disabled { reason: "x".into() },
            UpdateStatus::Idle,
            UpdateStatus::Checking,
            UpdateStatus::Downloading {
                version: "0.2.0".into(),
                percent: 50,
            },
            UpdateStatus::UpToDate {
                version: "0.1.0".into(),
            },
            UpdateStatus::Failed {
                error: UpdateError::Network("x".into()),
            },
        ] {
            assert_eq!(entry.follow_status(Lang::En, &hidden), EntryChange::Keep);
            assert!(!entry.is_shown(), "{hidden:?} shows nothing");
        }
        assert!(matches!(
            entry.follow_status(Lang::En, &ready("0.2.0")),
            EntryChange::Show(_)
        ));
        // A check, then a download of a newer version: the entry stays.
        assert_eq!(
            entry.follow_status(Lang::En, &UpdateStatus::Checking),
            EntryChange::Keep
        );
        assert_eq!(
            entry.follow_status(
                Lang::En,
                &UpdateStatus::Downloading {
                    version: "0.2.1".into(),
                    percent: 10
                }
            ),
            EntryChange::Keep
        );
        assert!(entry.is_shown());
        assert_eq!(
            entry.follow_status(Lang::En, &ready("0.2.1")),
            EntryChange::Retext("Restart to update to 0.2.1".into())
        );
        // Settled states without a package hide it.
        for settled in [
            UpdateStatus::UpToDate {
                version: "0.2.1".into(),
            },
            UpdateStatus::Failed {
                error: UpdateError::FeedNotFound,
            },
            UpdateStatus::Idle,
            UpdateStatus::Disabled { reason: "x".into() },
        ] {
            entry.follow_status(Lang::En, &ready("0.2.1"));
            assert_eq!(
                entry.follow_status(Lang::En, &settled),
                EntryChange::Hide,
                "{settled:?}"
            );
        }
    }

    #[test]
    fn the_restart_entry_is_greyed_while_the_updater_is_busy() {
        assert!(!RestartEntry::clickable(&UpdateStatus::Checking));
        assert!(!RestartEntry::clickable(&UpdateStatus::Downloading {
            version: "0.2.1".into(),
            percent: 3
        }));
        assert!(RestartEntry::clickable(&UpdateStatus::Ready {
            version: "0.2.0".into()
        }));
    }

    #[test]
    fn update_entries_read_as_decided() {
        assert_eq!(label(Lang::En, Label::CheckForUpdates), "Check for updates");
        assert_eq!(
            label(Lang::Fr, Label::CheckForUpdates),
            "Rechercher des mises à jour"
        );
        assert_eq!(
            restart_to_update(Lang::En, "0.2.0"),
            "Restart to update to 0.2.0"
        );
        assert_eq!(
            restart_to_update(Lang::Fr, "0.3.0-beta.1"),
            "Redémarrer pour passer à la 0.3.0-beta.1"
        );
    }

    #[test]
    fn tooltip_names_the_helper_and_its_status() {
        let state = TrayState {
            page_connected: true,
            ..TrayState::default()
        };
        assert_eq!(state.tooltip(Lang::En), "Faulk helper: Page connected");
        assert_eq!(state.tooltip(Lang::Fr), "Helper Faulk : Page connectée");
        // The Windows tooltip holds 127 characters.
        for lang in LANGS {
            assert!(state.tooltip(lang).chars().count() < 128);
        }
    }

    #[test]
    fn code_entry_shows_the_current_code_spaced() {
        let state = TrayState {
            code: "04718263".into(),
            ..TrayState::default()
        };
        assert_eq!(
            state.code_entry(Lang::En),
            "Pairing code: 0471 8263 (click to copy)"
        );
        assert!(state.code_entry(Lang::Fr).contains("0471 8263"));
        assert_eq!(spaced_code("123"), "123");
    }

    #[test]
    fn pairing_url_puts_the_code_in_the_fragment_only() {
        assert_eq!(
            pairing_url("http://localhost:5173/", "12345678"),
            "http://localhost:5173/#pair=12345678"
        );
        assert_eq!(
            pairing_url("https://faulk.app/app?x=1#old", "12345678"),
            "https://faulk.app/app?x=1#pair=12345678"
        );
        assert_eq!(
            pairing_url(&app_url(false), "12345678"),
            "http://localhost:5173/app#pair=12345678"
        );
        let url = pairing_url(&app_url(true), "87654321");
        let (before, fragment) = url.split_once('#').unwrap();
        assert!(!before.contains("87654321"));
        assert_eq!(fragment, "pair=87654321");
    }

    #[test]
    fn app_url_is_origin_plus_path() {
        assert_eq!(app_url(false), "http://localhost:5173/app");
    }

    #[test]
    fn arguments_with_quotes_or_trailing_backslashes_survive_the_split() {
        assert_eq!(quote_arg(r"C:\my dir\"), r#""C:\my dir\\""#);
        assert_eq!(quote_arg(r#"say "hi""#), r#""say \"hi\"""#);
        assert_eq!(quote_arg(r#"a\"b"#), r#""a\\\"b""#);
        assert_eq!(quote_arg(r"C:\no\space"), r"C:\no\space");
        assert_eq!(quote_arg(r"C:\a b\c"), r#""C:\a b\c""#);
    }

    #[test]
    fn run_command_quotes_the_exe_and_spaced_arguments() {
        assert_eq!(
            run_command(r"C:\Program Files\Faulk\faulk-helper.exe", &[]),
            r#""C:\Program Files\Faulk\faulk-helper.exe""#
        );
        let args = ["--port", "47801", "--app-url", "http://a b/", ""].map(String::from);
        assert_eq!(
            run_command(r"C:\f.exe", &args),
            r#""C:\f.exe" --port 47801 --app-url "http://a b/" """#
        );
    }
}
