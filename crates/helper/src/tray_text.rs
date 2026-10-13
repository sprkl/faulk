//! What the tray icon shows, as pure functions: texts in English and French, the
//! helper's state as the user sees it, the link that opens Faulk and pairs it, and the
//! command line stored for "start with Windows".
//!
//! Kept free of any platform API so it compiles and is tested everywhere.

/// Origin of the page the tray opens. Change it when Faulk is served elsewhere.
pub const APP_ORIGIN: &str = "http://localhost:5173";
/// Path of the DAW on that origin. Becomes `/app` when a landing page takes `/`.
pub const APP_PATH: &str = "/";

/// The page the tray opens unless `--app-url` says otherwise.
pub fn default_app_url() -> String {
    format!("{APP_ORIGIN}{APP_PATH}")
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
        (Lang::En, Label::Quit) => "Quit",
        (Lang::Fr, Label::Quit) => "Quitter",
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
/// the helper was started with (each quoted when it holds a space or is empty).
pub fn run_command(exe: &str, args: &[String]) -> String {
    let mut line = format!("\"{exe}\"");
    for arg in args {
        line.push(' ');
        if arg.is_empty() || arg.contains([' ', '\t']) {
            line.push('"');
            line.push_str(arg);
            line.push('"');
        } else {
            line.push_str(arg);
        }
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    const LANGS: [Lang; 2] = [Lang::En, Lang::Fr];

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
        for l in [Label::OpenFaulk, Label::StartAtLogin, Label::Quit] {
            for lang in LANGS {
                assert!(!label(lang, l).is_empty());
            }
            assert_ne!(label(Lang::En, l), label(Lang::Fr, l));
        }
        assert_ne!(startup_failed(Lang::En), startup_failed(Lang::Fr));
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
        let url = pairing_url(&default_app_url(), "87654321");
        let (before, fragment) = url.split_once('#').unwrap();
        assert!(!before.contains("87654321"));
        assert_eq!(fragment, "pair=87654321");
    }

    #[test]
    fn default_app_url_is_origin_plus_path() {
        assert_eq!(default_app_url(), "http://localhost:5173/");
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
