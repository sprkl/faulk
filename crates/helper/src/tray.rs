//! The tray icon on Windows, and the few Windows services around it: UI language,
//! clipboard, the "start with Windows" registry value and the error message box.
//!
//! The tray lives on the main thread, which runs a Win32 message loop; the server runs
//! on tokio's threads and reaches the loop through a channel plus a thread message that
//! wakes it. Nothing here touches the audio thread.

use crate::server::HelperEvent;
use crate::tray_text::{Label, Lang, TrayState, label, lang_from_langid, pairing_url};
use std::sync::mpsc::Receiver;
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIconBuilder, TrayIconEvent};
use windows_sys::Win32::Foundation::{GlobalFree, HWND};
use windows_sys::Win32::Globalization::GetUserDefaultUILanguage;
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows_sys::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows_sys::Win32::System::Ole::CF_UNICODETEXT;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, GetSystemMetrics, MB_ICONERROR, MB_OK, MSG, MessageBoxW,
    PostThreadMessageW, SM_CXSMICON, SetTimer, TranslateMessage, WM_APP,
};
use winreg::RegKey;
use winreg::enums::HKEY_CURRENT_USER;

/// Resource ordinal of the icon embedded by `build.rs`.
const ICON_RESOURCE: u16 = 1;
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
/// Name of the value under the Run key.
pub const RUN_VALUE: &str = "FaulkHelper";

/// Language of the Windows user interface.
pub fn ui_lang() -> Lang {
    // SAFETY: no arguments, no preconditions.
    lang_from_langid(unsafe { GetUserDefaultUILanguage() })
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Shows a blocking error box (used when the helper cannot start: no console).
pub fn error_box(title: &str, text: &str) {
    let (title, text) = (wide(title), wide(text));
    // SAFETY: both strings are NUL-terminated UTF-16 buffers that outlive the call.
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}

/// Puts `text` on the clipboard as Unicode text.
pub fn copy_to_clipboard(text: &str) -> Result<(), String> {
    let data = wide(text);
    let bytes = data.len() * 2;
    // SAFETY: the clipboard is opened and closed on this thread around the calls. The
    // global block is allocated with `bytes` bytes and written with exactly `bytes`
    // bytes while locked. Once SetClipboardData succeeds the system owns the block;
    // on failure it is freed here.
    unsafe {
        if OpenClipboard(std::ptr::null_mut() as HWND) == 0 {
            return Err("cannot open the clipboard".into());
        }
        let result = (|| {
            EmptyClipboard();
            let block = GlobalAlloc(GMEM_MOVEABLE, bytes);
            if block.is_null() {
                return Err("cannot allocate the clipboard text".to_owned());
            }
            let target = GlobalLock(block) as *mut u16;
            if target.is_null() {
                GlobalFree(block);
                return Err("cannot lock the clipboard text".to_owned());
            }
            std::ptr::copy_nonoverlapping(data.as_ptr(), target, data.len());
            GlobalUnlock(block);
            if SetClipboardData(u32::from(CF_UNICODETEXT), block).is_null() {
                GlobalFree(block);
                return Err("cannot set the clipboard text".to_owned());
            }
            Ok(())
        })();
        CloseClipboard();
        result
    }
}

/// Whether this helper, started this way, starts with the user's session. A value
/// left by a moved executable or other arguments counts as off, so checking the box
/// rewrites it.
pub fn start_at_login(command: &str) -> bool {
    run_value_is(RUN_VALUE, command)
}

/// Stores `command` under the Run key, or removes the value when `None`.
pub fn set_start_at_login(command: Option<&str>) -> std::io::Result<()> {
    set_run_value(RUN_VALUE, command)
}

/// For the uninstaller: removes the Run value when it starts this executable, and says so
/// in the log file (the uninstall hook runs before `main` opens it, so it is opened here).
pub fn forget_own_start_at_login() {
    if let Some(dir) = dirs::config_dir() {
        let _ = crate::log::init(&dir.join("faulk").join("logs"), false);
    }
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    match forget_start_at_login(&exe.display().to_string()) {
        Ok(true) => crate::log!("uninstall: start with Windows removed"),
        Ok(false) => {}
        Err(e) => crate::log!("uninstall: cannot remove start with Windows: {e}"),
    }
}

/// Removes the Run value when it starts `exe`; returns whether it did. Called by the
/// uninstaller, which deletes the executable the value would start. A value that starts
/// another copy of the helper is left alone.
pub fn forget_start_at_login(exe: &str) -> std::io::Result<bool> {
    forget_run_value(RUN_VALUE, exe)
}

fn forget_run_value(name: &str, exe: &str) -> std::io::Result<bool> {
    let stored = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(RUN_KEY)
        .and_then(|key| key.get_value::<String, _>(name));
    match stored {
        Ok(stored) if crate::tray_text::run_command_starts(&stored, exe) => {
            set_run_value(name, None).map(|()| true)
        }
        _ => Ok(false),
    }
}

fn run_value_is(name: &str, command: &str) -> bool {
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(RUN_KEY)
        .and_then(|key| key.get_value::<String, _>(name))
        .is_ok_and(|stored| stored == command)
}

fn set_run_value(name: &str, command: Option<&str>) -> std::io::Result<()> {
    let (key, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(RUN_KEY)?;
    match command {
        Some(command) => key.set_value(name, &command),
        None => match key.delete_value(name) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            other => other,
        },
    }
}

/// Wakes the tray loop from another thread.
#[derive(Clone, Copy)]
pub struct Waker(u32);

impl Waker {
    /// A waker for the calling thread, which must be the one that runs [`run`].
    pub fn for_current_thread() -> Self {
        // SAFETY: no arguments, no preconditions.
        Self(unsafe { GetCurrentThreadId() })
    }

    pub fn wake(self) {
        // SAFETY: posting a message with no pointers to a thread id; if the thread has
        // no message queue yet the call fails harmlessly and the loop drains the
        // channel when it starts.
        unsafe {
            PostThreadMessageW(self.0, WM_APP, 0, 0);
        }
    }
}

/// What the tray needs from the helper.
pub struct TrayConfig {
    pub lang: Lang,
    pub app_url: String,
    /// The command line stored for "start with Windows".
    pub run_command: String,
    pub events: Receiver<HelperEvent>,
}

/// Shows the tray icon and runs the message loop until the user picks Quit.
/// Fails without a loop when the icon cannot be created.
pub fn run(config: TrayConfig) -> Result<(), String> {
    let lang = config.lang;
    let mut state = TrayState::default();

    let status = MenuItem::new(state.status(lang), false, None);
    let code = MenuItem::new(state.code_entry(lang), true, None);
    let open = MenuItem::new(label(lang, Label::OpenFaulk), true, None);
    let login = CheckMenuItem::new(
        label(lang, Label::StartAtLogin),
        true,
        start_at_login(&config.run_command),
        None,
    );
    let quit = MenuItem::new(label(lang, Label::Quit), true, None);
    let menu = Menu::new();
    menu.append_items(&[
        &status,
        &code,
        &PredefinedMenuItem::separator(),
        &open,
        &login,
        &PredefinedMenuItem::separator(),
        &quit,
    ])
    .map_err(|e| format!("cannot build the tray menu: {e}"))?;

    // SAFETY: no arguments, no preconditions.
    let size = unsafe { GetSystemMetrics(SM_CXSMICON) }.max(16) as u32;
    let icon = Icon::from_resource(ICON_RESOURCE, Some((size, size)))
        .map_err(|e| format!("cannot load the tray icon: {e}"))?;
    // Mouse events over the icon are not used; without a handler they would pile up
    // in an unbounded channel.
    TrayIconEvent::set_event_handler(Some(|_| {}));
    let tray = TrayIconBuilder::new()
        .with_icon(icon)
        .with_tooltip(state.tooltip(lang))
        .with_menu(Box::new(menu))
        .build()
        .map_err(|e| format!("cannot create the tray icon: {e}"))?;

    let refresh = |state: &TrayState| {
        status.set_text(state.status(lang));
        code.set_text(state.code_entry(lang));
        let _ = tray.set_tooltip(Some(state.tooltip(lang)));
    };
    let apply_events = |state: &mut TrayState| {
        let mut changed = false;
        for event in config.events.try_iter() {
            changed = true;
            match event {
                HelperEvent::Code(c) => state.code = c,
                HelperEvent::Session(on) => state.page_connected = on,
                HelperEvent::AudioError => state.audio_error = true,
            }
        }
        changed
    };
    apply_events(&mut state);
    refresh(&state);
    crate::log!("tray icon shown");
    // Windows drops thread messages while a modal loop runs (the open menu, a message
    // box), so a wake-up can be lost. A thread timer makes the loop look at the channel
    // at least every half second anyway.
    // SAFETY: a thread timer (no window, no callback); it posts WM_TIMER to this thread.
    unsafe {
        SetTimer(std::ptr::null_mut(), 0, 500, None);
    }

    // SAFETY: MSG is a plain C struct; all-zero is a valid value.
    let mut msg: MSG = unsafe { std::mem::zeroed() };
    loop {
        // SAFETY: `msg` is a valid, writable MSG for the whole call.
        let got = unsafe { GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) };
        if got <= 0 {
            break; // WM_QUIT or an error: stop like Quit.
        }
        // SAFETY: `msg` was filled by GetMessageW just above.
        unsafe {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        if apply_events(&mut state) {
            refresh(&state);
        }
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            if event.id == quit.id() {
                crate::log!("quit from the tray");
                // The icon goes away when `tray` drops, on return.
                return Ok(());
            } else if event.id == open.id() {
                let url = pairing_url(&config.app_url, &state.code);
                if let Err(e) = webbrowser::open(&url) {
                    crate::log!("cannot open the browser: {e}");
                }
            } else if event.id == code.id() {
                if let Err(e) = copy_to_clipboard(&state.code) {
                    crate::log!("{e}");
                }
            } else if event.id == login.id() {
                let wanted = login.is_checked();
                let result = set_start_at_login(wanted.then_some(config.run_command.as_str()));
                if let Err(e) = result {
                    crate::log!("cannot change the start at login setting: {e}");
                }
                // Show what the registry holds, whatever happened.
                login.set_checked(start_at_login(&config.run_command));
                crate::log!("start at login: {}", login.is_checked());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_run_value_is_written_compared_and_removed() {
        // A throwaway value next to the real one, removed at the end.
        let name = format!("FaulkHelperTest{}", std::process::id());
        let command = r#""C:\Faulk\faulk-helper.exe" --port 47801"#;
        assert!(!run_value_is(&name, command), "absent reads as off");
        set_run_value(&name, Some(command)).unwrap();
        assert!(run_value_is(&name, command), "the same command reads as on");
        assert!(
            !run_value_is(&name, r#""C:\Elsewhere\faulk-helper.exe""#),
            "another command reads as off"
        );
        set_run_value(&name, None).unwrap();
        assert!(!run_value_is(&name, command), "removed reads as off");
        set_run_value(&name, None).expect("removing an absent value is fine");
    }

    #[test]
    fn the_uninstaller_forgets_only_the_value_of_its_own_executable() {
        let name = format!("FaulkHelperForgetTest{}", std::process::id());
        let installed = r"C:\Users\u\AppData\Local\FaulkHelper\current\faulk-helper.exe";
        let dev = r#""C:\git\music\target\release\faulk-helper.exe" --port 47801"#;
        set_run_value(&name, Some(dev)).unwrap();
        assert!(!forget_run_value(&name, installed).unwrap(), "another copy");
        assert!(run_value_is(&name, dev), "kept");
        let own = format!("\"{installed}\" --port 47801");
        set_run_value(&name, Some(&own)).unwrap();
        assert!(forget_run_value(&name, installed).unwrap(), "its own");
        assert!(!run_value_is(&name, &own), "removed");
        assert!(
            !forget_run_value(&name, installed).unwrap(),
            "absent: nothing to do"
        );
    }
}
