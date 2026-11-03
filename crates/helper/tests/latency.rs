//! Controller to sound in Boost (#598), measured on one clock: from the instant a note is
//! sent to the first block the helper's engine renders with sound. The engine is rendered by a
//! thread at the pace of an audio callback of 128 frames at 48 kHz; the output device's own
//! latency (its buffers, the converter) is not in the figure. The page is a stand-in on a real
//! session over WebSocket (the page's core, `Synced`): idle, or busy 20 ms out of every 30 ms
//! without reading its socket (long tasks of the main thread). Two paths: `direct` (the page
//! sent `live.routing`: the helper plays the note, #598) and `page` (no table: the note goes to
//! the page and comes back as engine bytes, the path before #598).
//!
//! The note enters through a MIDI loopback when the system's MIDI API shows one (one port out,
//! the same loop's port in, opened by the helper's real `MidiManager`): `FAULK_MIDI_LOOP_OUT`
//! and `FAULK_MIDI_LOOP_IN` name a part of the ports' names ("Loopback A" by default; the ports
//! seen are printed). Without one, the test thread hands each message to the helper's input
//! (`MidiSink::receive`, what midir's callback calls) and says so: the figure then leaves out
//! the MIDI driver. Ignored (it takes about two minutes); run with
//! `cargo test -p helper --release --test latency -- --ignored --nocapture`.

use futures_util::{SinkExt, StreamExt};
use helper::audio::{EngineSlot, Meter, audio_core, spawn_flusher};
use helper::live::LiveInput;
use helper::midi::{MidiManager, MidiSink};
use helper::security::Policy;
use helper::server::{Shared, serve};
use protocol::{ClientMessage, ServerMessage};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

const ORIGIN: &str = "http://localhost:5173";
const TOKEN: &str = "0123456789abcdef0123456789abcdef";
const RATE: f32 = 48_000.0;
const FRAMES: usize = 128;
const NOTES: usize = 60;

/// Nanoseconds since `epoch` of the first block with sound since the last arm; 0 while none.
struct Sound {
    epoch: Instant,
    armed: AtomicBool,
    at: AtomicU64,
}

/// Where the notes come from: a loopback port (the helper opens its other end), or the test
/// thread handing them to the helper's input.
enum Controller {
    Loopback {
        out: midir::MidiOutputConnection,
        input: String,
    },
    Direct,
}

/// The loopback named by the environment, if the system shows it.
fn loopback() -> Option<Controller> {
    let out_name = std::env::var("FAULK_MIDI_LOOP_OUT").unwrap_or_else(|_| "Loopback A".into());
    let in_name = std::env::var("FAULK_MIDI_LOOP_IN").unwrap_or_else(|_| "Loopback A".into());
    let midi_out = midir::MidiOutput::new("faulk-latency-out").ok()?;
    let outs = midi_out.ports();
    let names: Vec<String> = outs
        .iter()
        .map(|p| midi_out.port_name(p).unwrap_or_default())
        .collect();
    println!("MIDI outputs: {names:?}");
    let midi_in = midir::MidiInput::new("faulk-latency-in").ok()?;
    let ins = midi_in.ports();
    let in_names: Vec<String> = ins
        .iter()
        .map(|p| midi_in.port_name(p).unwrap_or_default())
        .collect();
    println!("MIDI inputs: {in_names:?}");
    let out_port = names.iter().position(|n| n.contains(&out_name))?;
    let in_port = in_names.iter().position(|n| n.contains(&in_name))?;
    let out = midi_out.connect(&outs[out_port], "faulk-latency").ok()?;
    Some(Controller::Loopback {
        out,
        input: ins[in_port].id(),
    })
}

#[test]
#[ignore = "takes about two minutes; run by hand (module doc)"]
fn controller_to_sound_in_boost() {
    let mut controller = loopback().unwrap_or(Controller::Direct);
    let source = match &controller {
        Controller::Loopback { .. } => "through a MIDI loopback",
        Controller::Direct => {
            "no MIDI loopback seen: handed to the helper's input (MIDI driver left out)"
        }
    };
    let mut report = Vec::new();
    for (path, busy) in [
        ("direct", false),
        ("direct", true),
        ("page", false),
        ("page", true),
    ] {
        let figures = run(&mut controller, path == "direct", busy);
        report.push(format!(
            "{path:6} page {:4}: {}",
            if busy { "busy" } else { "idle" },
            figures
        ));
    }
    println!(
        "controller to sound, {NOTES} notes each, callback of {FRAMES} frames at 48 kHz, {source}:"
    );
    for line in report {
        println!("  {line}");
    }
}

/// One series: a fresh helper, a page stand-in, `NOTES` notes. The figures as text.
fn run(controller: &mut Controller, direct: bool, busy: bool) -> String {
    let input = match controller {
        Controller::Loopback { input, .. } => Some(input.clone()),
        Controller::Direct => None,
    };
    let meter = Arc::new(Meter::default());
    let (mut core, sender) = audio_core(RATE, meter.clone()).unwrap();
    let engine = Arc::new(EngineSlot::default());
    engine.install(sender, RATE);
    let _flusher = spawn_flusher(&engine);
    let sound = Arc::new(Sound {
        epoch: Instant::now(),
        armed: AtomicBool::new(false),
        at: AtomicU64::new(0),
    });
    let stop = Arc::new(AtomicBool::new(false));

    // The audio callback: a block every 128 frames of time, the first with sound noted.
    let audio = {
        let (sound, stop) = (sound.clone(), stop.clone());
        std::thread::spawn(move || {
            let period = Duration::from_secs_f64(FRAMES as f64 / f64::from(RATE));
            let mut out = vec![0.0f32; FRAMES * 2];
            let mut next = Instant::now();
            while !stop.load(Ordering::Relaxed) {
                core.process(&mut out, 2);
                if sound.armed.load(Ordering::Acquire) && out.iter().any(|s| s.abs() > 1e-4) {
                    let at = sound.epoch.elapsed().as_nanos() as u64;
                    sound.at.store(at.max(1), Ordering::Release);
                    sound.armed.store(false, Ordering::Release);
                }
                next += period;
                while Instant::now() < next {
                    std::hint::spin_loop();
                }
            }
        })
    };

    // The helper's server, on its own runtime.
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let listener = runtime
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let (midi_tx, midi_rx) = tokio::sync::mpsc::unbounded_channel();
    let sink = MidiSink::new(Arc::new(LiveInput::new(engine.clone())), midi_tx);
    let key = sink.clone();
    let mut send = |message: &[u8]| match controller {
        Controller::Loopback { out, .. } => out.send(message).unwrap(),
        Controller::Direct => key.receive(message),
    };
    let shared = Arc::new(Shared::new(
        Policy {
            port,
            allowed_origins: vec![ORIGIN.into()],
            token: TOKEN.into(),
        },
        engine.clone(),
        MidiManager::new(sink),
        midi_rx,
        meter,
        Box::new(|_| {}),
    ));
    runtime.spawn(serve(listener, shared));

    // The page stand-in, on its own thread (its "main thread"): a busy page blocks it.
    let ready = Arc::new(Mutex::new(false));
    let page = {
        let (stop, ready, input) = (stop.clone(), ready.clone(), input.clone());
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(page_stand_in(port, input, direct, busy, &stop, &ready));
        })
    };
    while !*ready.lock().unwrap() {
        std::thread::sleep(Duration::from_millis(10));
    }
    std::thread::sleep(Duration::from_millis(200));

    let mut latencies = Vec::new();
    for i in 0..NOTES {
        // Struck at a different point of the page's busy cycle each time.
        std::thread::sleep(Duration::from_millis(5 + (i as u64 * 7) % 30));
        sound.at.store(0, Ordering::Release);
        sound.armed.store(true, Ordering::Release);
        let sent = sound.epoch.elapsed().as_nanos() as u64;
        send(&[0x90, 60, 100]);
        let deadline = Instant::now() + Duration::from_secs(2);
        while sound.at.load(Ordering::Acquire) == 0 && Instant::now() < deadline {
            std::thread::yield_now();
        }
        let at = sound.at.load(Ordering::Acquire);
        send(&[0x80, 60, 0]);
        if at != 0 {
            latencies.push(at.saturating_sub(sent) as f64 / 1e6);
        }
        // The release (0.3 s by default) ends before the next note.
        std::thread::sleep(Duration::from_millis(450));
    }
    stop.store(true, Ordering::Relaxed);
    audio.join().unwrap();
    page.join().unwrap();
    // The session's end lets go of the input before the next series opens it.
    std::thread::sleep(Duration::from_millis(300));
    drop(runtime);

    let lost = NOTES - latencies.len();
    latencies.sort_by(f64::total_cmp);
    let q = |q: f64| latencies[((q * latencies.len() as f64) as usize).min(latencies.len() - 1)];
    if latencies.is_empty() {
        return format!("no note sounded ({lost} lost)");
    }
    format!(
        "min {:.2} ms, median {:.2} ms, p95 {:.2} ms, max {:.2} ms ({} notes, {lost} silent)",
        latencies[0],
        q(0.5),
        q(0.95),
        latencies[latencies.len() - 1],
        latencies.len()
    )
}

/// The page: a session, its snapshot, the helper's input selected (a loopback), the table when `direct`;
/// then it runs every `midi.message` the helper did not play and sends its bytes back. When
/// `busy`, it blocks 20 ms out of every 30 without reading its socket.
async fn page_stand_in(
    port: u16,
    input: Option<String>,
    direct: bool,
    busy: bool,
    stop: &AtomicBool,
    ready: &Mutex<bool>,
) {
    let mut request = format!("ws://127.0.0.1:{port}/session?token={TOKEN}")
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("origin", ORIGIN.parse().unwrap());
    let (mut ws, _) = connect_async(request).await.unwrap();
    let epoch = loop {
        if let Some(Ok(Message::Text(text))) = ws.next().await
            && let Ok(ServerMessage::Hello { epoch, .. }) = serde_json::from_str(&text)
        {
            break epoch;
        }
    };
    let mut page = commands::engine_msgs::Synced::single_synth(1);
    let select = serde_json::json!({
        "type": "view.select_track",
        "track": protocol::single_synth::TRACK,
    });
    page.run(&serde_json::from_value(select).unwrap()).unwrap();
    let snapshot = page.resync_after(epoch).unwrap();
    ws.send(Message::binary(snapshot)).await.unwrap();
    if direct {
        let routing = commands::host::LiveRouting::of(&page.state, &page.ctx.engine, page.epoch());
        let message = ClientMessage::LiveRouting {
            epoch: routing.epoch,
            tracks: routing.tracks,
        };
        ws.send(Message::text(serde_json::to_string(&message).unwrap()))
            .await
            .unwrap();
    }
    if let Some(id) = input {
        let select = ClientMessage::MidiSelectInput { id: Some(id) };
        ws.send(Message::text(serde_json::to_string(&select).unwrap()))
            .await
            .unwrap();
    }
    *ready.lock().unwrap() = true;

    let start = Instant::now();
    while !stop.load(Ordering::Relaxed) {
        if busy && start.elapsed().as_millis() % 30 < 20 {
            // A long task: the thread does not read its socket.
            let until = Instant::now()
                + Duration::from_millis(20 - (start.elapsed().as_millis() % 30) as u64);
            while Instant::now() < until {
                std::hint::spin_loop();
            }
            continue;
        }
        let Ok(Some(Ok(message))) = tokio::time::timeout(Duration::from_millis(1), ws.next()).await
        else {
            continue;
        };
        let Message::Text(text) = message else {
            continue;
        };
        let Ok(ServerMessage::MidiMessage { data, played }) = serde_json::from_str(&text) else {
            continue;
        };
        if played {
            continue;
        }
        let command = match data[0] & 0xF0 {
            0x90 if data[2] > 0 => serde_json::json!({
                "type": "live.input_note_on", "note": data[1], "velocity": data[2],
            }),
            0x80 | 0x90 => serde_json::json!({ "type": "live.input_note_off", "note": data[1] }),
            _ => continue,
        };
        let (_, bytes) = page.run(&serde_json::from_value(command).unwrap()).unwrap();
        if !bytes.is_empty() {
            ws.send(Message::binary(bytes)).await.unwrap();
        }
    }
    let _ = ws.close(None).await;
}
