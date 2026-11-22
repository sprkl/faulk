//! A Sluice listening to another track (#158), built by commands and rendered by the export
//! (`engine::offline`): a held pad gated by a hi-hat track playing every eighth note (the hat
//! muted: it still triggers). The pad is silent between two hats and sounds on each, the same
//! natively and in WASM, and the project saved and reopened exports the same. The render is
//! written to `.qa/renders/sluice-gated-pad.wav` for the ear. (The device itself is tested in
//! `crates/engine/tests/it/sluice.rs`.)

use commands::export::ExportSettings;
use commands::testing::{context, fixture};
use commands::{Command, Output, State, clip, device, mixer, track};
use project::file::{MemFs, OpenOptions, SaveOptions, SaveState, open, save};
use project::{DeviceId, Note, Tick, TrackId, TrackKind};
use test_support::Session;
use test_support::export::{export_native, export_wasm};

/// Bars of the song, four beats each at 120 BPM: an eighth note is 0.25 s.
const BARS: i64 = 4;
const EIGHTH: f32 = 0.25;

fn note(start: i64, pitch: u8, length: i64, velocity: u8) -> Note {
    Note {
        start: Tick(start),
        pitch,
        length: Tick(length),
        velocity,
        cents: 0,
        chance: Note::DEFAULT_CHANCE,
        repeat: Note::DEFAULT_REPEAT,
    }
}

fn clip_with(s: &mut Session, track: &TrackId, notes: Vec<Note>) {
    let Output::Clip(clip::Output::Create { clip, .. }) =
        s.exec(Command::Clip(clip::Command::Create {
            track: track.clone(),
            at: clip::ClipTarget::Arrangement { start: Tick(0) },
            length: Some(Tick(BARS * 3840)),
            name: None,
        }))
    else {
        panic!("a clip")
    };
    s.run(Command::Clip(clip::Command::NotesAdd { clip, notes }));
}

fn set(s: &mut Session, fx: &DeviceId, param: &str, value: f64) {
    s.run(Command::Device(device::Command::SetParam {
        device: fx.clone(),
        param: param.into(),
        value,
    }));
}

/// The song: the fixture's synth track as a pad holding a chord over every bar, with a Sluice
/// (fast, silent when closed), and a drum track playing a closed hat on every eighth note,
/// muted. `listening` sets the Sluice's source to the hat; without it the Sluice listens to the
/// pad itself, which is loud and keeps it open (the reference: the pad as it is).
fn song(listening: bool) -> (State, TrackId, TrackId, DeviceId) {
    let f = fixture();
    let mut s = Session::from_state(f.state.clone(), context());
    let pad = f.track.clone();
    let Output::Track(track::Output::Create { track: hat, .. }) =
        s.exec(Command::Track(track::Command::Create {
            kind: TrackKind::Midi,
            instrument: Some("faulk.drums".into()),
            name: Some("Hat".into()),
            color: None,
        }))
    else {
        panic!("a track")
    };
    let chord = (0..BARS)
        .flat_map(|bar| [60u8, 64, 67].map(|p| note(bar * 3840, p, 3840, 70)))
        .collect();
    clip_with(&mut s, &pad, chord);
    let hats = (0..BARS * 8).map(|b| note(b * 480, 42, 120, 110)).collect();
    clip_with(&mut s, &hat, hats);
    let Output::Device(device::Output::Insert { device: gate }) =
        s.exec(Command::Device(device::Command::Insert {
            track: Some(pad.clone()),
            master: false,
            kind: "faulk.sluice".into(),
            index: 0,
        }))
    else {
        panic!("a gate")
    };
    for (param, value) in [
        ("threshold", -50.0),
        ("close", -60.0),
        ("attack", 0.0005),
        ("hold", 0.04),
        ("release", 0.02),
    ] {
        set(&mut s, &gate, param, value);
    }
    s.run(Command::Mixer(mixer::Command::SetMute {
        track: hat.clone(),
        mute: true,
    }));
    if listening {
        s.run(Command::Device(device::Command::SetSidechain {
            device: gate.clone(),
            source: Some(hat.clone()),
        }));
    }
    (s.state().clone(), pad, hat, gate)
}

fn db(x: f32) -> f32 {
    20.0 * x.max(1e-9).log10()
}

#[test]
fn a_pad_is_gated_by_a_hat_and_silent_between_two() {
    let (own_state, ..) = song(false);
    let (state, ..) = song(true);
    let own = export_native(&own_state, ExportSettings::default()).render();
    let gated = export_native(&state, ExportSettings::default());
    let r = gated.render();
    r.assert_clean();
    r.assert_no_denormals();
    assert_eq!(gated.silenced, 0);
    r.write_wav("sluice-gated-pad");
    for step in 1..BARS as usize * 8 - 1 {
        let t = step as f32 * EIGHTH;
        // Just after a hat (10 to 50 ms: the hold keeps it open for 40 ms): the pad sounds
        // as loud as the pad alone.
        let with_hat = db(r.rms_in(t + 0.01..t + 0.05));
        let alone = db(own.rms_in(t + 0.01..t + 0.05));
        assert!(
            (with_hat - alone).abs() < 1.5,
            "step {step}: {with_hat:.1} dB gated, {alone:.1} dB alone"
        );
        // Late in the step (150 to 240 ms: the hold and the release are over): silence.
        let between = r.peak_in(t + 0.15..t + EIGHTH - 0.01);
        assert!(between == 0.0, "step {step}: {between} between two hats");
    }
}

#[test]
fn the_gated_pad_is_the_same_natively_and_in_wasm() {
    let (state, ..) = song(true);
    let native = export_native(&state, ExportSettings::default()).render();
    let wasm = export_wasm(&state, ExportSettings::default()).render();
    let largest = native
        .left
        .iter()
        .zip(&wasm.left)
        .chain(native.right.iter().zip(&wasm.right))
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    eprintln!("native/WASM: largest difference {largest:e}");
    assert_eq!(native.frames(), wasm.frames());
    assert!(largest <= test_support::PARITY_TOLERANCE, "{largest:e}");
}

#[test]
fn the_source_is_saved_reopened_and_exported_the_same() {
    let (state, pad, hat, gate) = song(true);
    let mut fs = MemFs::new();
    save(
        &mut fs,
        &state.project,
        &mut SaveState::new(),
        None,
        &SaveOptions::default(),
    )
    .expect("the project saves");
    let project = open(&fs, &OpenOptions::default())
        .expect("the project opens")
        .project;
    assert_eq!(project.tracks[&pad].devices[&gate].sidechain, Some(hat));
    assert_eq!(project, state.project);
    let mut opened = state.clone();
    opened.project = project;
    let before = export_native(&state, ExportSettings::default()).render();
    let after = export_native(&opened, ExportSettings::default()).render();
    assert_eq!(before.left, after.left);
    assert_eq!(before.right, after.right);
}
