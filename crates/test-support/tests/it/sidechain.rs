//! A Compressor listening to another track (#952), built by commands and rendered by the
//! export (`engine::offline`): a held pad compressed by a kick on every beat goes down by at
//! least the expected reduction on each beat and comes back up between two; the kick muted
//! (it still triggers), the same natively and in WASM, and the source saved, reopened and
//! exported the same. The render is written to `.qa/renders/sidechain-pump.wav` for the ear.

use commands::export::ExportSettings;
use commands::testing::{context, fixture};
use commands::{Command, Output, State, clip, device, mixer, track};
use project::file::{MemFs, OpenOptions, SaveOptions, SaveState, open, save};
use project::{DeviceId, Note, Tick, TrackId, TrackKind};
use test_support::Session;
use test_support::export::{export_native, export_wasm};

/// Bars of the song, four beats each at 120 BPM: a beat is 0.5 s.
const BARS: i64 = 4;
const BEAT: f32 = 0.5;

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

/// The song: the fixture's synth track as a pad holding a chord over every bar, with a
/// Compressor (fast, hard: it shows the pump), and a drum track playing the kick on every
/// beat, muted. `listening` sets the Compressor's source to the kick.
fn song(listening: bool) -> (State, TrackId, TrackId, DeviceId) {
    let f = fixture();
    let mut s = Session::from_state(f.state.clone(), context());
    let pad = f.track.clone();
    let Output::Track(track::Output::Create { track: kick, .. }) =
        s.exec(Command::Track(track::Command::Create {
            kind: TrackKind::Midi,
            instrument: Some("faulk.drums".into()),
            name: Some("Kick".into()),
            color: None,
        }))
    else {
        panic!("a track")
    };
    let chord = (0..BARS)
        .flat_map(|bar| [60u8, 64, 67].map(|p| note(bar * 3840, p, 3840, 70)))
        .collect();
    clip_with(&mut s, &pad, chord);
    let kicks = (0..BARS * 4).map(|b| note(b * 960, 36, 240, 120)).collect();
    clip_with(&mut s, &kick, kicks);
    let Output::Device(device::Output::Insert { device: comp }) =
        s.exec(Command::Device(device::Command::Insert {
            track: Some(pad.clone()),
            master: false,
            kind: "faulk.compressor".into(),
            index: 0,
        }))
    else {
        panic!("a compressor")
    };
    for (param, value) in [
        ("threshold", -36.0),
        ("ratio", 10.0),
        ("knee", 0.0),
        ("attack", 0.001),
        ("release", 0.08),
    ] {
        s.run(Command::Device(device::Command::SetParam {
            device: comp.clone(),
            param: param.into(),
            value,
        }));
    }
    s.run(Command::Mixer(mixer::Command::SetMute {
        track: kick.clone(),
        mute: true,
    }));
    if listening {
        s.run(Command::Device(device::Command::SetSidechain {
            device: comp.clone(),
            source: Some(kick.clone()),
        }));
    } else {
        // The reference: the pad as it is, the Compressor bypassed.
        s.run(Command::Device(device::Command::SetBypass {
            device: comp.clone(),
            bypass: true,
        }));
    }
    (s.state().clone(), pad, kick, comp)
}

fn db(x: f32) -> f32 {
    20.0 * x.max(1e-9).log10()
}

/// Per beat: the pad's level just after the kick (20 to 60 ms) and just before the next one
/// (the last 60 ms of the beat), in dB.
fn per_beat(r: &test_support::Render) -> Vec<(f32, f32)> {
    (0..BARS * 4)
        .map(|b| {
            let t = b as f32 * BEAT;
            (
                db(r.rms_in(t + 0.02..t + 0.06)),
                db(r.rms_in(t + BEAT - 0.06..t + BEAT)),
            )
        })
        .collect()
}

#[test]
fn a_pad_pumps_on_every_kick_and_comes_back_between_two() {
    let (dry_state, ..) = song(false);
    let (state, ..) = song(true);
    let dry = export_native(&dry_state, ExportSettings::default()).render();
    let pumped = export_native(&state, ExportSettings::default());
    let r = pumped.render();
    r.assert_clean();
    r.assert_no_denormals();
    assert_eq!(pumped.silenced, 0);
    r.write_wav("sidechain-pump");
    // The pad alone (the kick is muted): the render with the Compressor bypassed is the
    // reference level of each beat.
    let (reference, beats) = (per_beat(&dry), per_beat(&r));
    eprintln!("per beat (dry, pumped): {reference:?} {beats:?}");
    for (b, ((dry_hit, dry_rest), (hit, rest))) in reference.iter().zip(&beats).enumerate() {
        let dip = dry_hit - hit;
        assert!(
            dip >= 12.0,
            "beat {b}: the pad goes down by at least 12 dB on the kick, {dip:.1} dB"
        );
        assert!(
            (dry_rest - rest).abs() < 1.5,
            "beat {b}: the pad comes back up before the next kick, {rest:.1} dB against {dry_rest:.1} dB"
        );
    }
    eprintln!("per beat (dry, pumped): {reference:?} {beats:?}");
}

#[test]
fn the_pump_is_the_same_natively_and_in_wasm() {
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
    let (state, pad, kick, comp) = song(true);
    // Through the folder format and back.
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
    assert_eq!(project.tracks[&pad].devices[&comp].sidechain, Some(kick));
    assert_eq!(project, state.project);
    let mut opened = state.clone();
    opened.project = project;
    let before = export_native(&state, ExportSettings::default()).render();
    let after = export_native(&opened, ExportSettings::default()).render();
    assert_eq!(before.left, after.left);
    assert_eq!(before.right, after.right);
}
