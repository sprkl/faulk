//! #700: an effect inserted by command into a running session (after the session's snapshot)
//! plays as the export of the same project renders it, sample for sample, natively and in
//! WASM, for each of the four effects.
//!
//! The gap #700 saw was the harness's: the session ran on a fresh examples' context, whose ids
//! start from the seed the song's own ids came from, so the inserted effect took the id of the
//! lead's synth and replaced it in the project (the export had no lead) while the engine was
//! told nothing (the playback kept it). `Session::from_state` now moves its context's ids past
//! the state's.

use commands::export::{ExportBitDepth, ExportSettings};
use commands::testing::context;
use commands::{Command, Output, device, transport};
use project::DeviceRole;
use test_support::export::{demo_song, export_native, export_wasm};
use test_support::{
    PARITY_TOLERANCE, QUANTUM, Render, SAMPLE_RATE, Scenario, Session, render_native, render_wasm,
};

const EFFECTS: [&str; 4] = [
    "faulk.compressor",
    "faulk.echo",
    "faulk.reverb",
    "faulk.filter",
];

/// The demo song opened as a session, then `kind` inserted on its lead by command: the lead
/// keeps its synth, followed by the new effect.
fn session_with(kind: Option<&str>) -> Session {
    let mut s = Session::from_state(demo_song(2, None), context());
    if let Some(kind) = kind {
        let lead = s
            .state()
            .project
            .tracks
            .iter()
            .find(|(_, t)| t.name == "Lead")
            .map(|(id, _)| id.clone())
            .expect("the lead");
        let out = s.exec(Command::Device(device::Command::Insert {
            track: Some(lead.clone()),
            master: false,
            kind: kind.into(),
            index: 0,
        }));
        let Output::Device(device::Output::Insert { device: fx }) = out else {
            panic!("{out:?}")
        };
        let chain: Vec<_> = s.state().project.tracks[&lead]
            .devices_in_order()
            .into_iter()
            .map(|(id, d)| (id.clone(), d.role))
            .collect();
        assert_eq!(chain.len(), 2, "{kind}: {chain:?}");
        assert_eq!(chain[0].1, DeviceRole::Instrument, "{kind}: {chain:?}");
        assert_eq!(chain[1], (fx, DeviceRole::Effect), "{kind}: {chain:?}");
    }
    s
}

/// Plays the session: half a second stopped, then from bar 1; the `n` frames from the start.
fn play(s: &mut Session, n: usize, render: impl Fn(&Scenario) -> Render) -> Render {
    let before = (0.5 * SAMPLE_RATE / QUANTUM as f32).ceil() as usize * QUANTUM as usize;
    s.render_secs(0.5);
    s.run(Command::Transport(transport::Command::Play {}));
    s.render_secs(n as f32 / SAMPLE_RATE + 0.1);
    let played = render(s.scenario());
    Render::new(
        played.sample_rate,
        played.left[before..before + n].to_vec(),
        played.right[before..before + n].to_vec(),
    )
}

/// Checks the song with `kind` inserted live and returns the body of its exported file.
fn check(kind: Option<&str>) -> Render {
    let settings = ExportSettings {
        bit_depth: ExportBitDepth::Float32,
        ..ExportSettings::default()
    };
    let mut s = session_with(kind);
    let exported = export_native(s.state(), settings);
    let n = exported.body_frames as usize;
    let file = exported.render();
    let body = Render::new(
        file.sample_rate,
        file.left[..n].to_vec(),
        file.right[..n].to_vec(),
    );
    body.assert_clean();
    body.assert_audible(0.2..3.5, 0.01);
    let played = play(&mut s, n, render_native);
    let (diff, at) = body.max_difference(&played);
    assert_eq!(
        diff, 0.0,
        "{kind:?}: the file differs from the playback by {diff} at frame {at}"
    );
    let wasm = export_wasm(s.state(), settings);
    assert_eq!(wasm.body_frames, exported.body_frames);
    let (d, at) = wasm.render().max_difference(&file);
    assert!(
        d <= PARITY_TOLERANCE,
        "{kind:?}: the WASM export differs by {d} at {at}"
    );
    let played_wasm = play(&mut session_with(kind), n, render_wasm);
    let (d, at) = played_wasm.max_difference(&body);
    assert!(
        d <= PARITY_TOLERANCE,
        "{kind:?}: the WASM playback differs from the file by {d} at {at}"
    );
    body
}

#[test]
fn the_song_without_an_effect_plays_as_it_exports() {
    check(None);
}

/// Each effect is heard (its file differs from the song's without it), so the equality of the
/// playback and the file also proves the live insert reached the engine.
#[test]
fn an_effect_inserted_live_plays_as_the_export_renders_it() {
    let dry = check(None);
    for kind in EFFECTS {
        let wet = check(Some(kind));
        let (d, _) = wet.max_difference(&dry);
        assert!(d > 0.01, "{kind}: the effect is not heard ({d})");
    }
}
