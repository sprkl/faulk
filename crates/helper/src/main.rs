//! `faulk-helper`: runs the engine on the default audio output and serves the page.
//!
//! Usage: `faulk-helper [--port 47800] [--allow-origin <origin>]...`

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use helper::audio::{Meter, audio_core};
use helper::midi::MidiManager;
use helper::security::{Policy, load_or_create_token};
use helper::server::{Shared, serve};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};

#[cfg(debug_assertions)]
#[global_allocator]
static ALLOCATOR: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

const DEFAULT_PORT: u16 = 47800;
const DEFAULT_ORIGINS: [&str; 2] = ["http://localhost:5173", "http://127.0.0.1:5173"];

struct Args {
    port: u16,
    origins: Vec<String>,
}

fn args() -> Result<Args, String> {
    let mut port = DEFAULT_PORT;
    let mut origins = Vec::new();
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--port" => {
                port = it
                    .next()
                    .and_then(|p| p.parse().ok())
                    .ok_or("--port needs a number")?;
            }
            "--allow-origin" => origins.push(it.next().ok_or("--allow-origin needs an origin")?),
            other => return Err(format!("unknown argument {other}")),
        }
    }
    if origins.is_empty() {
        origins = DEFAULT_ORIGINS.iter().map(|s| (*s).to_owned()).collect();
    }
    Ok(Args { port, origins })
}

#[tokio::main]
async fn main() {
    let args = args().unwrap_or_else(|e| {
        eprintln!("{e}\nusage: faulk-helper [--port 47800] [--allow-origin <origin>]...");
        std::process::exit(2);
    });

    let config_dir = dirs::config_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("faulk");
    let token = load_or_create_token(&config_dir).unwrap_or_else(|e| {
        eprintln!(
            "cannot store the session token in {}: {e}",
            config_dir.display()
        );
        std::process::exit(1);
    });

    // Audio output.
    let host = cpal::default_host();
    let device = host.default_output_device().unwrap_or_else(|| {
        eprintln!("no audio output device");
        std::process::exit(1);
    });
    let supported = device.default_output_config().unwrap_or_else(|e| {
        eprintln!("cannot read the output configuration: {e}");
        std::process::exit(1);
    });
    if supported.sample_format() != cpal::SampleFormat::F32 {
        eprintln!(
            "the default output uses {:?} samples; only f32 is supported for now",
            supported.sample_format()
        );
        std::process::exit(1);
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
            move |err| {
                eprintln!("audio stream error: {err}");
                stream_error.store(true, Ordering::Relaxed);
            },
            None,
        )
        .unwrap_or_else(|e| {
            eprintln!("cannot open the audio output: {e}");
            std::process::exit(1);
        });
    stream.play().unwrap_or_else(|e| {
        eprintln!("cannot start the audio output: {e}");
        std::process::exit(1);
    });

    // MIDI and server.
    let sender = Arc::new(Mutex::new(sender));
    let (activity_tx, activity_rx) = mpsc::channel();
    let mut midi = MidiManager::new(sender.clone(), activity_tx);
    midi.refresh();

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", args.port))
        .await
        .unwrap_or_else(|e| {
            eprintln!("cannot listen on 127.0.0.1:{}: {e}", args.port);
            std::process::exit(1);
        });
    let device_name = device
        .description()
        .map(|d| d.name().to_owned())
        .unwrap_or_else(|_| "default output".into());
    println!(
        "Faulk helper on 127.0.0.1:{} · {} · {} Hz, {} channels",
        args.port, device_name, sample_rate, channels
    );
    println!("Allowed origins: {}", args.origins.join(", "));

    let shared = Arc::new(Shared::new(
        Policy {
            port: args.port,
            allowed_origins: args.origins,
            token,
        },
        sender,
        midi,
        activity_rx,
        meter,
        sample_rate,
        audio_error,
        Box::new(|code| println!("Pairing code: {} {}", &code[..4], &code[4..])),
    ));
    serve(listener, shared).await;
    drop(stream);
}
