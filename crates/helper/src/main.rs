//! `faulk-helper`: runs the engine on the default audio output and serves the page.
//!
//! Usage: `faulk-helper [--port 47800] [--allow-origin <origin>]... [--app-url <url>]`
//!
//! On Windows the helper lives in the notification area (tray): status, pairing code,
//! "Open Faulk", start with Windows, Quit. A release build has no console; its log is
//! `<config dir>/faulk/logs/helper.log` (`%APPDATA%\faulk\logs` on Windows). On other
//! systems there is no tray yet: the helper runs in its terminal and prints the code.
//!
//! One helper per port: a second one finds the port taken by a Faulk helper, opens the
//! page and exits.

// Release builds on Windows start without a console window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use helper::audio::{EngineEvent, Meter, audio_core};
use helper::instance::{self, Startup};
use helper::log;
use helper::midi::MidiManager;
use helper::security::{Policy, load_or_create_token};
use helper::server::{HelperEvent, Shared, serve};
use helper::tray_text::{self, Lang};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

#[cfg(debug_assertions)]
#[global_allocator]
static ALLOCATOR: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

const DEFAULT_PORT: u16 = 47800;
const DEFAULT_ORIGINS: [&str; 2] = ["http://localhost:5173", "http://127.0.0.1:5173"];
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

/// Logs why the helper cannot start, tells the user, and exits.
fn fatal(lang: Lang, detail: &str) -> ! {
    log!("{detail}");
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
    let _ = lang;
    std::process::exit(1);
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

fn main() {
    let raw_args: Vec<String> = std::env::args().skip(1).collect();
    let lang = ui_lang();
    let config_dir = dirs::config_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("faulk");
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
    if let Err(e) = std_listener.set_nonblocking(true) {
        fatal(lang, &format!("cannot configure the listener: {e}"));
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
    let device = host
        .default_output_device()
        .unwrap_or_else(|| fatal(lang, "no audio output device"));
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
    let (mut core, sender) = audio_core(sample_rate, meter.clone());
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
    let listener = {
        let _guard = runtime.enter();
        tokio::net::TcpListener::from_std(std_listener)
            .unwrap_or_else(|e| fatal(lang, &format!("cannot listen: {e}")))
    };
    let sender = Arc::new(Mutex::new(sender));
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

    let shared = Arc::new(Shared::new(
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
    ));
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
    if let Ok(mut sender) = sender.lock() {
        sender.send(EngineEvent::AllNotesOff);
    }
    let deadline = Instant::now() + QUIT_RELEASE_LIMIT;
    while meter.take().1 > 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    drop(stream);
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
        assert_eq!(a.app_url, "http://localhost:5173/");
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

    #[test]
    fn refuses_bad_arguments() {
        assert!(args(&["--port"]).is_err());
        assert!(args(&["--port", "x"]).is_err());
        assert!(args(&["--app-url"]).is_err());
        assert!(args(&["--what"]).is_err());
    }
}
