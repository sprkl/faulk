//! Hiccup (#689) through the command layer and the engine: `device.insert` puts `faulk.hiccup`
//! after the drums of a MIDI track; `device.set_param` sets each of its parameters by name
//! (schema, description and undo checked by `commands::testing::check`); a drum loop played by
//! the transport through it at two settings (a stutter at every bar line, and a roll at chance
//! 50 % with every effect of the repeats on) plays as the export of the same project renders
//! it, sample for sample, natively and in WASM. At 120 BPM, a 1/16 slice taken at every bar
//! line repeats at the expected frames, to the sample. The renders are written to `.qa/689/`
//! for the ear. (The device itself is tested in `crates/engine/tests/it/hiccup.rs`.)

use std::path::PathBuf;

use commands::export::{ExportBitDepth, ExportSettings};
use commands::testing::{check, context, fixture};
use commands::{Command, Output, clip, device, track, transport};
use engine::devices::hiccup::{self, DESCRIPTOR};
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
        kind: "faulk.hiccup".into(),
        index: 0,
    }));
    let Output::Device(device::Output::Insert { device: fx }) = out else {
        panic!("{out:?}")
    };
    fx
}

/// The fixture's track with the drums as its instrument and a loop of `BARS` bars at 120 BPM
/// in the arrangement: kick, snare, closed hats on the eighths, an open hat at each bar's end.
fn drum_loop() -> (Session, TrackId) {
    let f = fixture();
    let mut s = Session::from_state(f.state.clone(), context());
    s.run(Command::Track(track::Command::SetInstrument {
        track: f.track.clone(),
        instrument: "faulk.drums".into(),
    }));
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
    let note = |start: i64, pitch: u8, velocity: u8| Note {
        start: Tick(start),
        pitch,
        length: Tick(120),
        velocity,
        cents: 0,
        chance: Note::DEFAULT_CHANCE,
        repeat: Note::DEFAULT_REPEAT,
    };
    let mut notes = Vec::new();
    for bar in 0..BARS {
        let b = bar * 3840;
        for kick in [0, 1920, 2400] {
            notes.push(note(b + kick, 36, 115));
        }
        for snare in [960, 2880] {
            notes.push(note(b + snare, 38, 105));
        }
        for eighth in 0..7 {
            notes.push(note(b + eighth * 480, 42, 70 + (eighth % 2) as u8 * 20));
        }
        notes.push(note(b + 3360, 46, 90));
    }
    s.run(Command::Clip(clip::Command::NotesAdd { clip, notes }));
    (s, f.track)
}

/// The settings of a render.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Setting {
    /// No Hiccup.
    Dry,
    /// A 1/16 slice at every bar line, repeated for half a bar, in place of the sound.
    Stutter,
    /// A 1/32 slice every half note at chance 50 %, shorter each time, decaying, low-passed,
    /// raised a fifth, for the whole interval.
    Roll,
}

fn session(setting: Setting) -> Session {
    let (mut s, track) = drum_loop();
    if setting != Setting::Dry {
        let fx = insert(&mut s, &track);
        let params: &[(&str, f64)] = match setting {
            Setting::Stutter => &[("gate", 50.0)],
            _ => &[
                ("interval", f64::from(hiccup::EVERY_1_2)),
                ("size", f64::from(hiccup::SIZE_1_32)),
                ("chance", 50.0),
                ("seed", 3.0),
                ("gate", 100.0),
                ("variation", f64::from(hiccup::SHORTER)),
                ("decay", 15.0),
                ("filter", f64::from(hiccup::LOW_PASS)),
                ("pitch", 7.0),
            ],
        };
        for (param, value) in params {
            s.run(set(&fx, param, *value));
        }
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

/// Writes a render under `.qa/689/` at the repository's root.
fn write_qa(render: &Render, name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.qa/689");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{name}.wav"));
    std::fs::write(&path, render.wav()).unwrap();
    println!("wrote {}", path.display());
    path
}

/// The export of `setting`, its body checked against the playback (natively: equal; WASM
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
    body.assert_audible(0.0..7.5, 0.01);
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
    let (d, at) = played_wasm.max_difference(&played);
    assert!(
        d <= PARITY_TOLERANCE,
        "{setting:?}: the WASM playback differs from the native one by {d} at {at}"
    );
    println!("{setting:?}: export = playback, WASM within {d:e}");
    played
}

#[test]
fn hiccup_goes_after_the_drums_and_each_parameter_is_set_by_name_and_undone() {
    let (s0, track) = drum_loop();
    let insert_cmd = Command::Device(device::Command::Insert {
        track: Some(track.clone()),
        master: false,
        kind: "faulk.hiccup".into(),
        index: 0,
    });
    check(&insert_cmd, s0.state(), s0.ctx()).unwrap();
    let mut s = s0;
    let fx = insert(&mut s, &track);
    let chain = s.state().project.tracks[&track].devices_in_order();
    assert_eq!(chain[0].1.role, DeviceRole::Instrument, "the drums first");
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
    assert!(commands::testing::check_refusal(&set(&fx, "length", 1.0), s.state(), s.ctx()).is_ok());
}

#[test]
fn a_drum_loop_through_hiccup_plays_as_it_exports_natively_and_in_wasm() {
    let dry = played_as_exported(Setting::Dry);
    let stutter = played_as_exported(Setting::Stutter);
    let roll = played_as_exported(Setting::Roll);
    for (render, name) in [(&dry, "dry"), (&stutter, "stutter"), (&roll, "roll")] {
        render.assert_no_denormals();
        write_qa(render, &format!("drums-{name}"));
    }
    // Each setting is heard.
    assert!(stutter.max_difference(&dry).0 > 0.05);
    assert!(roll.max_difference(&dry).0 > 0.05);

    // At 120 BPM and 48 kHz a bar is 96 000 frames and a 1/16 slice 6 000: from each bar line
    // `b`, the slice plays as is, then each repeat is the slice again, sample for sample on its
    // plateau (after its 2 ms fade in, before its fade out), until half a bar.
    let fade = 96;
    for bar in 0..BARS as usize {
        let b = bar * 96_000;
        for i in b..b + 6_000 {
            assert!(
                (stutter.left[i] - dry.left[i]).abs() <= 1e-6,
                "slice, frame {i}"
            );
        }
        for k in 1..8 {
            let start = b + k * 6_000;
            for u in fade + 2..6_000 - fade - 2 {
                let (got, want) = (stutter.left[start + u], dry.left[b + u]);
                assert!(
                    (got - want).abs() <= 1e-6,
                    "bar {bar}, repeat {k}, frame {u}: {got} vs {want}"
                );
            }
        }
        // After half a bar, the loop as is.
        for i in b + 48_000 + 2..b + 96_000 {
            assert!(
                (stutter.left[i] - dry.left[i]).abs() <= 1e-6,
                "after the gate, frame {i}"
            );
        }
    }

    // The roll at chance 50 %: the half notes it captured are those its draw names.
    for n in 0..(2 * BARS) as usize {
        let at = n * 48_000;
        // Just after the slice (1/32: 3 000 frames), a captured point plays a repeat raised a
        // fifth, where the loop plays on otherwise.
        let window = at + 3_000 + 300..at + 6_000;
        let differs = window
            .clone()
            .any(|i| (roll.left[i] - dry.left[i]).abs() > 1e-4);
        assert_eq!(differs, hiccup::captures(3, n as i64, 50), "half note {n}");
    }
}
