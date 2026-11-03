//! BO-1's first criterion: the same message stream gives the same render natively and in
//! WASM. A page's project, built by commands (`test_support` scenarios: several tracks, clips,
//! the arrangement, automation), is sent to a real helper session as the page sends it: each
//! engine stream in a binary frame over a loopback WebSocket. The helper's audio core renders
//! it in callbacks of one render quantum, and the result is compared with the web build of the
//! engine rendering the same scenario (`render_wasm`) and with the native harness.
//!
//! The helper's render follows the scenario step by step: each stream is submitted before the
//! blocks that follow it in the scenario are rendered (the test waits until the session's
//! reader handed it to the engine), and the control thread's flush runs before each block, as
//! the harness does (`render_native`). The helper's own flusher is not running: the test is it.

use futures_util::SinkExt;
use helper::audio::{EngineSlot, Meter, audio_core};
use helper::live::LiveInput;
use helper::midi::{MidiManager, MidiSink};
use helper::security::Policy;
use helper::server::{Shared, serve};
use protocol::ServerMessage;
use std::sync::Arc;
use std::time::Duration;
use test_support::{PARITY_TOLERANCE, QUANTUM, Render, SAMPLE_RATE, Scenario, Step};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

const ORIGIN: &str = "http://localhost:5173";
const TOKEN: &str = "0123456789abcdef0123456789abcdef";

/// Renders `scenario` through a real helper session.
async fn render_through_the_helper(scenario: &Scenario) -> Render {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let meter = Arc::new(Meter::default());
    let (mut core, sender) = audio_core(SAMPLE_RATE, meter.clone()).unwrap();
    let engine = Arc::new(EngineSlot::default());
    engine.install(sender, SAMPLE_RATE);
    let (midi_out, midi_in) = tokio::sync::mpsc::unbounded_channel();
    let shared = Arc::new(Shared::new(
        Policy {
            port,
            allowed_origins: vec![ORIGIN.into()],
            token: TOKEN.into(),
            mcp_token: helper::security::McpToken::Fixed(TOKEN.into()),
        },
        engine.clone(),
        MidiManager::new(MidiSink::new(
            Arc::new(LiveInput::new(engine.clone())),
            midi_out,
        )),
        midi_in,
        meter,
        Box::new(|_| {}),
    ));
    tokio::spawn(serve(listener, shared));

    let mut request = format!("ws://127.0.0.1:{port}/session?token={TOKEN}")
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("Origin", ORIGIN.parse().unwrap());
    let (mut ws, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    // The hello: a fresh engine, in epoch 0, so the scenario's first `Reset` is newer.
    loop {
        use futures_util::StreamExt;
        let frame = ws.next().await.unwrap().unwrap();
        if let Message::Text(text) = frame {
            match serde_json::from_str::<ServerMessage>(&text).unwrap() {
                ServerMessage::Hello { epoch, .. } => {
                    assert_eq!(epoch, 0);
                    break;
                }
                other => panic!("expected hello, got {other:?}"),
            }
        }
    }

    let (mut left, mut right) = (Vec::new(), Vec::new());
    let mut out = vec![0.0f32; QUANTUM as usize * 2];
    let mut sent = 0u64;
    for step in &scenario.steps {
        match step {
            Step::Push(bytes) => {
                ws.send(Message::binary(bytes.clone())).await.unwrap();
                sent += 1;
                // Submitted before the blocks that follow it.
                let mut waited = 0;
                while engine.with(|s| s.received()) != Some(sent) {
                    tokio::time::sleep(Duration::from_millis(1)).await;
                    waited += 1;
                    assert!(waited < 5_000, "the stream never reached the engine");
                }
            }
            Step::Render(blocks) => {
                for _ in 0..*blocks {
                    engine.flush();
                    core.process(&mut out, 2);
                    for frame in out.chunks(2) {
                        left.push(frame[0]);
                        right.push(frame[1]);
                    }
                }
            }
        }
    }
    engine.flush();
    let (rejected, waiting, refusal) = engine
        .with(|s| (s.rejected(), s.waiting(), s.take_refusal()))
        .unwrap();
    assert_eq!(rejected, 0, "messages were rejected");
    assert_eq!(waiting, 0, "streams still wait");
    assert_eq!(refusal, None, "a stream was refused");
    Render::new(SAMPLE_RATE, left, right)
}

async fn assert_parity(name: &str, scenario: Scenario) {
    let helper = render_through_the_helper(&scenario).await;
    assert!(
        helper.peak() > 0.05,
        "{name}: the helper's render is not silent"
    );
    helper.assert_clean();
    let native = test_support::render_native(&scenario);
    // The same engine, the same blocks: the helper renders what the native harness renders.
    helper.assert_close(&native, 0.0);
    let wasm = test_support::render_wasm(&scenario);
    helper.assert_close(&wasm, PARITY_TOLERANCE);
}

#[tokio::test]
async fn two_tracks_of_clips_render_alike_through_the_helper_and_in_wasm() {
    assert_parity("two tracks", test_support::clips::two_tracks(true)).await;
}

#[tokio::test]
async fn an_arranged_song_with_automation_renders_alike_through_the_helper_and_in_wasm() {
    let song =
        test_support::arrangement::song_with(test_support::arrangement::Play::Straight, true);
    assert_parity("song", song).await;
}
