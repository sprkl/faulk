//! `faulk-helper`: runs the engine on the default audio output and serves the page.
//!
//! Usage: `faulk-helper [--port 47800] [--allow-origin <origin>]... [--app-url <url>]`
//!
//! By default the helper accepts the production site and the local dev servers, and
//! "Open Faulk" opens `https://www.faulk.app/app` in a release build and the dev server in
//! a debug build (`tray_text::app_url`).
//!
//! On Windows the helper lives in the notification area (tray): status, pairing code,
//! "Open Faulk", start with Windows, Quit. A release build has no console; its log is
//! `<config dir>/faulk/logs/helper.log` (`%APPDATA%\faulk\logs` on Windows). On other
//! systems there is no tray yet: the helper runs in its terminal and prints the code.
//!
//! One helper per port: a second one finds the port taken by a Faulk helper, opens the
//! page and exits.
//!
//! Once installed, the helper updates itself (`helper::updater`): Velopack's startup hook
//! runs first, applies an update downloaded by a previous run when no other helper is
//! running, and answers the installer's `--veloapp-*` arguments; an update thread then
//! checks the feed at start, about every hour and when a page connects, and downloads in the background.

// Release builds on Windows start without a console window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use helper::audio::{EventSender, Meter, audio_core, spawn_flusher};
use helper::instance::{self, Startup};
use helper::log;
use helper::midi::MidiManager;
use helper::midi::all_notes_off;
use helper::security::{Policy, load_or_create_token};
use helper::server::{HelperEvent, Shared, serve};
use helper::tray_text::{self, Lang};
use helper::updater::{self, Config, UpdateStatus, UpdateWatch, Updater};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::time::{Duration, Instant};

#[cfg(debug_assertions)]
#[global_allocator]
static ALLOCATOR: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

const DEFAULT_PORT: u16 = 47800;
/// The production site (the apex redirects to `www`, accepted in case that changes), then
/// the web app's dev server (`pnpm dev`, 5173) and its static preview (`pnpm preview`,
/// 4173). The same list in every build.
const DEFAULT_ORIGINS: [&str; 6] = [
    "https://www.faulk.app",
    "https://faulk.app",
    "http://localhost:5173",
    "http://127.0.0.1:5173",
    "http://localhost:4173",
    "http://127.0.0.1:4173",
];
/// Longest wait on Quit for the voices to end their release before the audio stops.
const QUIT_RELEASE_LIMIT: Duration = Duration::from_secs(2);
const USAGE: &str =
    "usage: faulk-helper [--port 47800] [--allow-origin <origin>]... [--app-url <url>]";

#[derive(Debug, PartialEq)]
struct Args {
    port: u16,
    origins: Vec<String>,
    /// The page "Open Faulk" opens.
    app_url: String,
}

fn parse_args(raw: &[String]) -> Result<Args, String> {
    let mut port = DEFAULT_PORT;
    let mut origins = Vec::new();
    let mut app_url = tray_text::default_app_url();
    let mut it = raw.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--port" => {
                port = it
                    .next()
                    .and_then(|p| p.parse().ok())
                    .ok_or("--port needs a number")?;
            }
            "--allow-origin" => {
                origins.push(it.next().ok_or("--allow-origin needs an origin")?.clone())
            }
            "--app-url" => app_url = it.next().ok_or("--app-url needs a URL")?.clone(),
            other => return Err(format!("unknown argument {other}")),
        }
    }
    if origins.is_empty() {
        origins = DEFAULT_ORIGINS.iter().map(|s| (*s).to_owned()).collect();
    }
    Ok(Args {
        port,
        origins,
        app_url,
    })
}

#[cfg(windows)]
fn ui_lang() -> Lang {
    helper::tray::ui_lang()
}

#[cfg(not(windows))]
fn ui_lang() -> Lang {
    let locale = std::env::var("LC_ALL")
        .or_else(|_| std::env::var("LANG"))
        .unwrap_or_default();
    if locale.starts_with("fr") {
        Lang::Fr
    } else {
        Lang::En
    }
}

/// The port a starting helper claimed, until the server takes it: a failed start frees it at
/// once (`fatal`), so that a helper launched meanwhile is not told the port is taken.
static CLAIMED: Mutex<Option<std::net::TcpListener>> = Mutex::new(None);
/// The updater, once started: a failed start lets its first check end (`fatal`).
static UPDATES: OnceLock<UpdateWatch> = OnceLock::new();
/// Longest wait of a failed start for the update check and download in progress.
const FATAL_UPDATE_LIMIT: Duration = Duration::from_secs(120);
/// A debug build reports no audio output device when this variable is set (startup tests).
const NO_AUDIO_ENV: &str = "FAULK_HELPER_TEST_NO_AUDIO";
/// A debug build pauses this many milliseconds where a release build shows its error dialog
/// (startup tests).
#[cfg(debug_assertions)]
const FATAL_PAUSE_ENV: &str = "FAULK_HELPER_TEST_FATAL_PAUSE_MS";

/// Logs why the helper cannot start, tells the user, and exits. A helper that cannot start can
/// still update itself, and the update may fix what failed: the port is freed first; while the
/// error is shown, the update check started at launch is let to end (bounded), then the instance
/// lock is let go, so that the next start applies a downloaded update even if the error is still
/// on screen; the exit waits for both.
fn fatal(lang: Lang, detail: &str) -> ! {
    log!("{detail}");
    drop(
        CLAIMED
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take(),
    );
    // While the error is shown: let the update check end, then stop holding the instance lock,
    // so that the next start can apply what was downloaded even if the error stays on screen.
    let settle_then_release = || {
        if let Some(updates) = UPDATES.get() {
            wait_for_update(updates, FATAL_UPDATE_LIMIT);
        }
        updater::release_instance_lock();
    };
    let settling = std::thread::Builder::new()
        .name("faulk-fatal-update".into())
        .spawn(settle_then_release)
        .ok();
    show_fatal(lang, detail);
    match settling {
        Some(thread) => {
            let _ = thread.join();
        }
        // No thread: the same, after the error.
        None => settle_then_release(),
    }
    std::process::exit(1);
}

/// Lets a check or download in progress end, at most `limit`, saying so in the journal.
fn wait_for_update(updates: &UpdateWatch, limit: Duration) {
    if updates.settle(Duration::ZERO) {
        return;
    }
    log!(
        "update: waiting up to {} s for the update check to end before exiting",
        limit.as_secs()
    );
    if !updates.settle(limit) {
        log!("update: still running, abandoned as the helper exits");
    }
}

/// The audio output, or `None` when there is none (or, in a debug build, when a test asks).
fn default_output(host: &cpal::Host) -> Option<cpal::Device> {
    if cfg!(debug_assertions) && std::env::var_os(NO_AUDIO_ENV).is_some() {
        return None;
    }
    host.default_output_device()
}

/// Tells the user why the helper stops: a dialog in a Windows release build (no console).
fn show_fatal(lang: Lang, detail: &str) {
    #[cfg(all(windows, not(debug_assertions)))]
    {
        let log_line = log::path()
            .map(|p| format!("\n\n{}", p.display()))
            .unwrap_or_default();
        helper::tray::error_box(
            "Faulk",
            &format!("{}\n\n{detail}{log_line}", tray_text::startup_failed(lang)),
        );
    }
    #[cfg(not(all(windows, not(debug_assertions))))]
    let _ = (lang, detail);
    // A debug build stands for the dialog with a pause when a test asks: the startup test checks
    // that the port is already free while the dialog is up.
    #[cfg(debug_assertions)]
    if let Some(ms) = std::env::var(FATAL_PAUSE_ENV)
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
    {
        std::thread::sleep(Duration::from_millis(ms));
    }
}

/// `scheme://host[:port]` of a URL, as a browser sends it in `Origin`.
fn origin_of(url: &str) -> &str {
    let after_scheme = url.find("://").map_or(0, |i| i + 3);
    let end = url[after_scheme..]
        .find(['/', '?', '#'])
        .map_or(url.len(), |i| after_scheme + i);
    &url[..end]
}

fn open_in_browser(url: &str) {
    if let Err(e) = webbrowser::open(url) {
        log!("cannot open the browser: {e}");
    }
}

/// Ends every note, then waits up to `limit` for the voices to finish their release.
/// Returns whether they did. A lock poisoned by a panic does not skip the note off.
fn release_all_voices(sender: &Mutex<EventSender>, meter: &Meter, limit: Duration) -> bool {
    sender
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .send(&all_notes_off());
    let deadline = Instant::now() + limit;
    loop {
        if meter.take().1 == 0 {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn main() {
    // First: the installer and the updater run the executable with `--veloapp-*` arguments
    // and expect it to exit at once, before any argument parsing, port claim or window.
    updater::startup();
    let raw_args: Vec<String> = std::env::args().skip(1).collect();
    let lang = ui_lang();
    let config_dir = helper::config_dir();
    // Only the helper that serves rotates the log: another instance must not move the
    // file the running one writes to.
    let open_log = |rotate: bool| {
        if let Err(e) = log::init(&config_dir.join("logs"), rotate) {
            eprintln!("cannot open the log file: {e}");
        }
    };
    let args = parse_args(&raw_args).unwrap_or_else(|e| {
        open_log(false);
        fatal(lang, &format!("{e}\n{USAGE}"))
    });

    // One helper per port: bind before anything else. A port taken by a Faulk helper
    // means it already runs: show the page and leave.
    let mut probe_origins = args.origins.clone();
    for origin in DEFAULT_ORIGINS {
        if !probe_origins.iter().any(|o| o == origin) {
            probe_origins.push(origin.to_owned());
        }
    }
    let std_listener = match instance::claim(args.port, &probe_origins, instance::PROBE_PATIENCE) {
        Startup::Serve(listener) => {
            open_log(true);
            listener
        }
        Startup::AlreadyRunning => {
            open_log(false);
            log!(
                "a Faulk helper already runs on port {}: opening Faulk",
                args.port
            );
            open_in_browser(&args.app_url);
            std::process::exit(0);
        }
        Startup::Blocked(reason) => {
            open_log(false);
            fatal(lang, &reason)
        }
    };
    *CLAIMED.lock().unwrap_or_else(|p| p.into_inner()) = Some(std_listener);

    // Updates of the installed helper, on their own thread; disabled when run from cargo.
    // Started before anything that can stop the start (token, audio output, runtime, tray):
    // a helper that cannot start still checks, and `fatal` lets that check end (#515).
    let updates = Updater::start(Config::from_env());
    if let UpdateStatus::Disabled { reason } = updates.status() {
        log!("update: disabled ({reason})");
    }
    let _ = UPDATES.set(updates.watch());

    if !args
        .origins
        .iter()
        .any(|o| origin_of(&args.app_url).eq_ignore_ascii_case(o))
    {
        log!(
            "warning: the page {} is not among the allowed origins; it will not be able to pair",
            args.app_url
        );
    }

    let token = load_or_create_token(&config_dir).unwrap_or_else(|e| {
        fatal(
            lang,
            &format!(
                "cannot store the session token in {}: {e}",
                config_dir.display()
            ),
        )
    });

    // Audio output.
    let host = cpal::default_host();
    let device = default_output(&host).unwrap_or_else(|| fatal(lang, "no audio output device"));
    let supported = device
        .default_output_config()
        .unwrap_or_else(|e| fatal(lang, &format!("cannot read the output configuration: {e}")));
    if supported.sample_format() != cpal::SampleFormat::F32 {
        fatal(
            lang,
            &format!(
                "the default output uses {:?} samples; only f32 is supported for now",
                supported.sample_format()
            ),
        );
    }
    let config: cpal::StreamConfig = supported.config();
    let sample_rate = config.sample_rate as f32;
    let channels = config.channels as usize;

    let meter = Arc::new(Meter::default());
    let (mut core, sender) = audio_core(sample_rate, meter.clone())
        .unwrap_or_else(|e| fatal(lang, &format!("cannot create the engine: {e:?}")));
    let audio_error = Arc::new(AtomicBool::new(false));
    let stream_error = audio_error.clone();
    let stream = device
        .build_output_stream(
            config,
            move |data: &mut [f32], info: &cpal::OutputCallbackInfo| {
                let at = info.timestamp();
                core.meter()
                    .set_output_delay(at.playback.saturating_duration_since(at.callback));
                #[cfg(debug_assertions)]
                assert_no_alloc::assert_no_alloc(|| core.process(data, channels));
                #[cfg(not(debug_assertions))]
                core.process(data, channels);
            },
            // May run on the stream's thread: only raise the flag; the server's ticker
            // reports it.
            move |_err| stream_error.store(true, Ordering::Relaxed),
            None,
        )
        .unwrap_or_else(|e| fatal(lang, &format!("cannot open the audio output: {e}")));
    stream
        .play()
        .unwrap_or_else(|e| fatal(lang, &format!("cannot start the audio output: {e}")));

    // MIDI and server.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap_or_else(|e| fatal(lang, &format!("cannot start the async runtime: {e}")));
    // Taken in its own statement: `fatal` locks `CLAIMED` too.
    let claimed = CLAIMED.lock().unwrap_or_else(|p| p.into_inner()).take();
    let std_listener = claimed.unwrap_or_else(|| fatal(lang, "the claimed port was released"));
    if let Err(e) = std_listener.set_nonblocking(true) {
        fatal(lang, &format!("cannot configure the listener: {e}"));
    }
    let listener = {
        let _guard = runtime.enter();
        tokio::net::TcpListener::from_std(std_listener)
            .unwrap_or_else(|e| fatal(lang, &format!("cannot listen: {e}")))
    };
    let sender = Arc::new(Mutex::new(sender));
    // The control thread: pushes what waited on the engine's ring and drops its garbage.
    let _flusher = spawn_flusher(&sender);
    let (activity_tx, activity_rx) = mpsc::channel();
    let mut midi = MidiManager::new(sender.clone(), activity_tx);
    midi.refresh();

    let device_name = device
        .description()
        .map(|d| d.name().to_owned())
        .unwrap_or_else(|_| "default output".into());
    log!(
        "Faulk helper on 127.0.0.1:{} · {} · {} Hz, {} channels",
        args.port,
        device_name,
        sample_rate,
        channels
    );
    log!("Allowed origins: {}", args.origins.join(", "));

    // The pairing code goes to the console and the tray, never to the log file.
    let print_code = |event: &HelperEvent| match event {
        HelperEvent::Code(code) => eprintln!("Pairing code: {}", tray_text::spaced_code(code)),
        HelperEvent::AudioError => log!("the audio output stream reported an error"),
        HelperEvent::Session(on) => log!("page session {}", if *on { "opened" } else { "closed" }),
    };
    #[cfg(windows)]
    let (tray_events, notify): (_, Box<dyn Fn(HelperEvent) + Send + Sync>) = {
        let (tx, rx) = mpsc::channel();
        let waker = helper::tray::Waker::for_current_thread();
        (
            rx,
            Box::new(move |event| {
                print_code(&event);
                let _ = tx.send(event);
                waker.wake();
            }),
        )
    };
    #[cfg(not(windows))]
    let notify: Box<dyn Fn(HelperEvent) + Send + Sync> = Box::new(move |event| print_code(&event));

    let shared = Arc::new(
        Shared::new(
            Policy {
                port: args.port,
                allowed_origins: args.origins.clone(),
                token,
            },
            sender.clone(),
            midi,
            activity_rx,
            meter.clone(),
            sample_rate,
            audio_error,
            notify,
        )
        .with_updates(updates.watch()),
    );
    runtime.spawn(serve(listener, shared));

    #[cfg(windows)]
    {
        let exe = std::env::current_exe()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let tray = helper::tray::run(helper::tray::TrayConfig {
            lang,
            app_url: args.app_url.clone(),
            run_command: tray_text::run_command(&exe, &raw_args),
            events: tray_events,
            updater: &updates,
        });
        if let Err(e) = tray {
            // Without an icon a release build could be neither seen nor quit: stop.
            runtime.shutdown_timeout(Duration::from_secs(1));
            drop(stream);
            fatal(lang, &e);
        }
    }
    #[cfg(not(windows))]
    runtime.block_on(std::future::pending::<()>());

    // Quit: close the pages' sessions first (they go back to Web mode and stop sending
    // notes), then let every voice finish its release, then stop the audio.
    runtime.shutdown_timeout(Duration::from_secs(1));
    if !release_all_voices(&sender, &meter, QUIT_RELEASE_LIMIT) {
        log!("voices still sounding when the audio stops");
    }
    drop(stream);
    // Does not wait: a check or a download in progress is abandoned with the process.
    drop(updates);
    log!("stopped");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Result<Args, String> {
        parse_args(&list.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>())
    }

    #[test]
    fn defaults() {
        let a = args(&[]).unwrap();
        assert_eq!(a.port, DEFAULT_PORT);
        assert_eq!(a.origins, DEFAULT_ORIGINS);
        // Tests run in a debug build unless `cargo test --release`.
        let expected = if cfg!(debug_assertions) {
            "http://localhost:5173/app"
        } else {
            "https://www.faulk.app/app"
        };
        assert_eq!(a.app_url, expected);
    }

    #[test]
    fn reads_port_origins_and_app_url() {
        let a = args(&[
            "--port",
            "47801",
            "--allow-origin",
            "http://localhost:4173",
            "--app-url",
            "http://localhost:4173/app",
        ])
        .unwrap();
        assert_eq!(a.port, 47801);
        assert_eq!(a.origins, ["http://localhost:4173"]);
        assert_eq!(a.app_url, "http://localhost:4173/app");
    }

    #[test]
    fn origin_of_keeps_scheme_host_and_port() {
        assert_eq!(origin_of("http://localhost:5173/"), "http://localhost:5173");
        assert_eq!(origin_of("http://localhost:5173"), "http://localhost:5173");
        assert_eq!(
            origin_of("https://faulk.app/app?x#pair=1"),
            "https://faulk.app"
        );
        assert_eq!(
            origin_of("http://127.0.0.1:5173#a"),
            "http://127.0.0.1:5173"
        );
    }

    /// A core with one note sounding, its sender and its meter.
    fn sounding_core() -> (
        helper::audio::AudioCore,
        Arc<Mutex<EventSender>>,
        Arc<Meter>,
    ) {
        let meter = Arc::new(Meter::default());
        let (mut core, mut sender) = audio_core(48_000.0, meter.clone()).unwrap();
        sender.send(&commands::Command::Live(commands::live::Command::NoteOn {
            track: project::TrackId::parse(protocol::single_synth::TRACK).unwrap(),
            note: 60,
            velocity: 100,
        }));
        let mut block = vec![0.0; 480 * 2];
        core.process(&mut block, 2);
        assert_eq!(meter.take().1, 1);
        (core, Arc::new(Mutex::new(sender)), meter)
    }

    /// Runs audio callbacks of 10 ms on a thread until `stop` is set.
    fn callbacks(
        mut core: helper::audio::AudioCore,
        stop: Arc<AtomicBool>,
    ) -> std::thread::JoinHandle<()> {
        std::thread::spawn(move || {
            let mut block = vec![0.0; 480 * 2];
            while !stop.load(Ordering::Relaxed) {
                core.process(&mut block, 2);
                std::thread::sleep(Duration::from_millis(1));
            }
        })
    }

    #[test]
    fn quit_ends_the_notes_and_waits_for_the_release() {
        let (core, sender, meter) = sounding_core();
        let stop = Arc::new(AtomicBool::new(false));
        let audio = callbacks(core, stop.clone());
        assert!(release_all_voices(&sender, &meter, Duration::from_secs(5)));
        assert_eq!(meter.take().1, 0);
        stop.store(true, Ordering::Relaxed);
        audio.join().unwrap();
    }

    #[test]
    fn quit_gives_up_on_a_stream_that_no_longer_runs() {
        let (_core, sender, meter) = sounding_core();
        let start = Instant::now();
        assert!(!release_all_voices(
            &sender,
            &meter,
            Duration::from_millis(100)
        ));
        let waited = start.elapsed();
        assert!(waited >= Duration::from_millis(100) && waited < Duration::from_secs(2));
    }

    #[test]
    fn quit_ends_the_notes_even_with_a_poisoned_lock() {
        let (core, sender, meter) = sounding_core();
        let poison = sender.clone();
        let _ = std::thread::spawn(move || {
            let _guard = poison.lock().unwrap();
            panic!("poison the sender lock");
        })
        .join();
        assert!(sender.is_poisoned());
        let stop = Arc::new(AtomicBool::new(false));
        let audio = callbacks(core, stop.clone());
        assert!(release_all_voices(&sender, &meter, Duration::from_secs(5)));
        stop.store(true, Ordering::Relaxed);
        audio.join().unwrap();
    }

    #[test]
    fn refuses_bad_arguments() {
        assert!(args(&["--port"]).is_err());
        assert!(args(&["--port", "x"]).is_err());
        assert!(args(&["--app-url"]).is_err());
        assert!(args(&["--what"]).is_err());
    }

    #[test]
    fn default_origins_cover_dev_and_preview_servers() {
        for origin in [
            "http://localhost:5173",
            "http://127.0.0.1:5173",
            "http://localhost:4173",
            "http://127.0.0.1:4173",
        ] {
            assert!(DEFAULT_ORIGINS.contains(&origin), "{origin}");
        }
    }

    #[test]
    fn default_origins_cover_the_production_site() {
        for origin in ["https://www.faulk.app", "https://faulk.app"] {
            assert!(DEFAULT_ORIGINS.contains(&origin), "{origin}");
        }
    }

    #[test]
    fn a_release_build_opens_the_site_and_a_debug_build_the_dev_server() {
        assert_eq!(tray_text::app_url(true), "https://www.faulk.app/app");
        assert_eq!(tray_text::app_url(false), "http://localhost:5173/app");
        assert_eq!(
            tray_text::default_app_url(),
            tray_text::app_url(!cfg!(debug_assertions))
        );
    }

    #[test]
    fn both_default_pages_can_pair_with_the_default_origins() {
        for release in [true, false] {
            let url = tray_text::app_url(release);
            assert!(DEFAULT_ORIGINS.contains(&origin_of(&url)), "{url}");
        }
    }

    #[test]
    fn flags_replace_the_production_defaults() {
        let a = args(&[
            "--allow-origin",
            "https://example.test",
            "--app-url",
            "https://example.test/app",
        ])
        .unwrap();
        assert_eq!(a.origins, ["https://example.test"]);
        assert_eq!(a.app_url, "https://example.test/app");
    }
}
