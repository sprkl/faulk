//! Ripple (#688) through the command layer and the engine: `device.insert` puts `faulk.ripple`
//! after the synth of a MIDI track; `device.set_param` sets each of its parameters by name
//! (schema, description and undo checked by `commands::testing::check`); a pad played by the
//! transport through it (dry, its slow phaser preset, its synced flanger preset, loaded with
//! `preset.load`) plays as the export of the same project renders it, sample for sample
//! natively and within the parity tolerance in WASM: the synced LFO sits at the same position
//! in the file as in the playback. The renders are written to `.qa/688/` for the ear. (The
//! device itself is tested in `crates/engine/tests/it/ripple.rs`.)

use std::path::PathBuf;

use commands::export::{ExportBitDepth, ExportSettings};
use commands::preset::PresetRef;
use commands::testing::{check, check_refusal, context, fixture};
use commands::{Command, Output, clip, device, preset, transport};
use engine::devices::ripple::DESCRIPTOR;
use project::preset::PresetLocation;
use project::{DeviceId, DeviceRole, Note, Tick, TrackId};
use test_support::export::{export_native, export_wasm};
use test_support::{
    PARITY_TOLERANCE, QUANTUM, Render, SAMPLE_RATE, Scenario, Session, render_native, render_wasm,
};

const BARS: i64 = 4;

fn set(device: &DeviceId, param: &str, value: f64) -> Command {
    Command::Device(device::Command::SetParam {
        device: device.clone(),
        param: param.into(),
        value,
    })
}

fn insert(s: &mut Session, track: &TrackId) -> DeviceId {
    let out = s.exec(Command::Device(device::Command::Insert {
        track: Some(track.clone()),
        master: false,
        kind: "faulk.ripple".into(),
        index: 0,
    }));
    let Output::Device(device::Output::Insert { device: fx }) = out else {
        panic!("{out:?}")
    };
    fx
}

/// The fixture's synth as a soft saw pad, with a clip of `BARS` bars at 120 BPM in the
/// arrangement: A minor 7 for two bars, F major 7 for two.
fn pad() -> (Session, TrackId, DeviceId) {
    let f = fixture();
    let mut s = Session::from_state(f.state.clone(), context());
    for (param, value) in [
        ("waveform", 1.0),
        ("attack", 0.25),
        ("decay", 0.5),
        ("sustain", 0.8),
        ("release", 0.6),
        ("gain", 0.3),
    ] {
        s.run(set(&f.device, param, value));
    }
    let Output::Clip(clip::Output::Create { clip, .. }) =
        s.exec(Command::Clip(clip::Command::Create {
            track: f.track.clone(),
            at: clip::ClipTarget::Arrangement { start: Tick(0) },
            length: Some(Tick(BARS * 3840)),
            name: None,
        }))
    else {
        panic!("a clip")
    };
    let note = |start: i64, pitch: u8| Note {
        start: Tick(start),
        pitch,
        length: Tick(2 * 3840 - 240),
        velocity: 90,
        cents: 0,
        chance: Note::DEFAULT_CHANCE,
        repeat: Note::DEFAULT_REPEAT,
    };
    let mut notes = Vec::new();
    for pitch in [45, 57, 60, 64, 67] {
        notes.push(note(0, pitch));
    }
    for pitch in [41, 53, 57, 60, 64] {
        notes.push(note(2 * 3840, pitch));
    }
    s.run(Command::Clip(clip::Command::NotesAdd { clip, notes }));
    (s, f.track, f.device)
}

/// The settings of a render.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Setting {
    Dry,
    SlowPhaser,
    SyncedFlanger,
}

fn session(setting: Setting) -> Session {
    let (mut s, track, _) = pad();
    let id = match setting {
        Setting::Dry => return s,
        Setting::SlowPhaser => "faulk.ripple/slow-phaser",
        Setting::SyncedFlanger => "faulk.ripple/synced-flanger",
    };
    let fx = insert(&mut s, &track);
    let out = s.exec(Command::Preset(preset::Command::Load {
        device: fx,
        preset: PresetRef {
            location: PresetLocation::Factory,
            id: id.into(),
        },
    }));
    let Output::Preset(preset::Output::Load {
        ignored,
        clamped,
        defaulted,
        ..
    }) = out
    else {
        panic!("{out:?}")
    };
    assert!(ignored.is_empty() && clamped.is_empty() && defaulted.is_empty());
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

/// Writes a render under `.qa/688/` at the repository's root.
fn write_qa(render: &Render, name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.qa/688");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{name}.wav"));
    std::fs::write(&path, render.wav()).unwrap();
    println!("wrote {}", path.display());
    path
}

/// The export of `setting`, its body checked against the playback (natively: equal; the WASM
/// export and playback: within the parity tolerance); returns the playback.
fn played_as_exported(setting: Setting) -> Render {
    let settings = ExportSettings {
        bit_depth: ExportBitDepth::Float32,
        ..ExportSettings::default()
    };
    let mut s = session(setting);
    let exported = export_native(s.state(), settings);
    let n = (BARS as usize * 2 * SAMPLE_RATE as usize).min(exported.body_frames as usize);
    let file = exported.render();
    let body = Render::new(
        file.sample_rate,
        file.left[..n].to_vec(),
        file.right[..n].to_vec(),
    );
    body.assert_clean();
    body.assert_audible(0.5..7.5, 0.01);
    let played = play(&mut s, n, render_native);
    let (diff, at) = body.max_difference(&played);
    assert_eq!(
        diff, 0.0,
        "{setting:?}: the file differs from the playback by {diff} at frame {at}"
    );
    let wasm = export_wasm(s.state(), settings);
    let wasm_file = wasm.render();
    let wasm_body = Render::new(
        wasm_file.sample_rate,
        wasm_file.left[..n].to_vec(),
        wasm_file.right[..n].to_vec(),
    );
    let (d, at) = wasm_body.max_difference(&body);
    assert!(
        d <= PARITY_TOLERANCE,
        "{setting:?}: the WASM export differs by {d} at {at}"
    );
    let played_wasm = play(&mut session(setting), n, render_wasm);
    let (dp, at) = played_wasm.max_difference(&played);
    assert!(
        dp <= PARITY_TOLERANCE,
        "{setting:?}: the WASM playback differs from the native one by {dp} at {at}"
    );
    println!(
        "{setting:?}: export = playback; WASM export within {d:e}, WASM playback within {dp:e}"
    );
    played
}

#[test]
fn ripple_goes_after_the_synth_and_each_parameter_is_set_by_name_and_undone() {
    let (s0, track, _) = pad();
    let insert_cmd = Command::Device(device::Command::Insert {
        track: Some(track.clone()),
        master: false,
        kind: "faulk.ripple".into(),
        index: 0,
    });
    check(&insert_cmd, s0.state(), s0.ctx()).unwrap();
    let mut s = s0;
    let fx = insert(&mut s, &track);
    let chain = s.state().project.tracks[&track].devices_in_order();
    assert_eq!(chain[0].1.role, DeviceRole::Instrument, "the synth first");
    assert_eq!(chain[1].0, &fx);
    assert_eq!(chain[1].1.role, DeviceRole::Effect);
    for p in DESCRIPTOR.params {
        for value in [p.min, p.max, p.min + (p.max - p.min) * 0.37] {
            let cmd = set(&fx, p.key, value);
            let current = s.state().project.tracks[&track].devices[&fx]
                .params
                .get(p.key)
                .copied()
                .unwrap_or(p.default);
            if p.sanitize_f64(value) == Some(current) {
                let other = if value == p.max { p.min } else { p.max };
                s.run(set(&fx, p.key, other));
            }
            check(&cmd, s.state(), s.ctx()).unwrap_or_else(|e| panic!("{} = {value}: {e}", p.key));
            let out = s.exec(cmd);
            let Output::Device(device::Output::SetParam { value: stored, .. }) = out else {
                panic!("{out:?}")
            };
            assert_eq!(Some(stored), p.sanitize_f64(value), "{} = {value}", p.key);
        }
    }
    assert!(check_refusal(&set(&fx, "resonance", 1.0), s.state(), s.ctx()).is_ok());
}

#[test]
fn a_pad_through_ripple_plays_as_it_exports_natively_and_in_wasm() {
    let dry = played_as_exported(Setting::Dry);
    let phaser = played_as_exported(Setting::SlowPhaser);
    let flanger = played_as_exported(Setting::SyncedFlanger);
    for (render, name) in [
        (&dry, "pad-dry"),
        (&phaser, "pad-slow-phaser"),
        (&flanger, "pad-synced-flanger"),
    ] {
        render.assert_no_denormals();
        write_qa(render, name);
    }
    // Each setting is heard, and moves: as the sweep carries the notches across a band, the
    // band's level against the dry pad's rises and falls (by eighth of a second over two
    // seconds: two cycles of the flanger's half-note sweep, a quarter of the phaser's 4 bars).
    assert!(phaser.max_difference(&dry).0 > 0.05);
    assert!(flanger.max_difference(&dry).0 > 0.05);
    let band_db = |r: &Render, w: usize| {
        let t = 0.5 + w as f32 / 8.0;
        let s = r.spectrum(t..t + 0.125);
        let e: f32 = s
            .magnitudes
            .iter()
            .enumerate()
            .filter(|(k, _)| (1_500.0..2_500.0).contains(&(*k as f32 * s.bin_hz)))
            .map(|(_, m)| m * m)
            .sum();
        10.0 * e.max(1e-20).log10()
    };
    for (render, name) in [(&phaser, "phaser"), (&flanger, "flanger")] {
        let levels: Vec<f32> = (0..16)
            .map(|w| band_db(render, w) - band_db(&dry, w))
            .collect();
        let (lo, hi) = levels
            .iter()
            .fold((f32::MAX, f32::MIN), |(l, h), v| (l.min(*v), h.max(*v)));
        println!("{name}: 1.5-2.5 kHz against the dry pad, by eighth of a second: {levels:.1?}");
        assert!(hi - lo > 3.0, "the {name} does not move: {levels:?}");
    }
}
