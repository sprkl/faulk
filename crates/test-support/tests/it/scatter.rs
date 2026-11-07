//! Scatter (#691) through the command layer and the engine: `project.set_scale` (F minor) and
//! `device.insert` put Scatter before the synth of a track; an arpeggio played from a clip comes
//! out with half its notes moved by one or two degrees, within the scale, the same natively and in
//! WASM and the same on a second render at equal seed, and is written to
//! `.qa/renders/scatter-arpeggio.wav` for the ear. A note held while Scatter is bypassed,
//! removed or changed, or through a stop, leaves nothing sounding. (The device itself is tested
//! in `crates/engine/tests/it/scatter.rs`.)

use commands::testing::{context, fixture};
use commands::{Command, Output, clip, device, live, project as p, transport};
use project::{DeviceId, Note, ScaleKind, Tick, TrackId};
use test_support::{Render, Session, parity, render_native};

fn set(s: &mut Session, fx: &DeviceId, param: &str, value: f64) {
    s.run(Command::Device(device::Command::SetParam {
        device: fx.clone(),
        param: param.into(),
        value,
    }));
}

/// A session on the fixture's synth track, in F minor, with Scatter before the synth.
fn session() -> (Session, DeviceId, TrackId) {
    let f = fixture();
    let mut s = Session::from_state(f.state.clone(), context());
    s.run(Command::Project(p::Command::SetScale {
        root: 5,
        mode: ScaleKind::NaturalMinor,
    }));
    let out = s.exec(Command::Device(device::Command::Insert {
        track: Some(f.track.clone()),
        master: false,
        kind: "faulk.scatter".into(),
        index: 0,
    }));
    let Output::Device(device::Output::Insert { device: fx }) = out else {
        panic!("{out:?}")
    };
    set(&mut s, &f.device, "release", 0.08);
    set(&mut s, &f.device, "gain", 0.5);
    (s, fx, f.track)
}

/// The arpeggio F4 A♭4 C5 E♭5, a sixteenth each at 120 BPM (240 ticks of 960 per beat), over
/// two bars, as a looping clip.
fn arpeggio(s: &mut Session, track: &TrackId) {
    let Output::Clip(clip::Output::Create { clip: c, .. }) =
        s.exec(Command::Clip(clip::Command::Create {
            track: track.clone(),
            at: clip::ClipTarget::Arrangement { start: Tick(0) },
            length: Some(Tick(7680)),
            name: None,
        }))
    else {
        panic!("a clip")
    };
    let notes = (0..32u32)
        .map(|i| Note {
            start: Tick(i64::from(i) * 240),
            pitch: [65u8, 68, 72, 75][i as usize % 4],
            length: Tick(200),
            velocity: 100,
            cents: 0,
            chance: Note::DEFAULT_CHANCE,
            repeat: Note::DEFAULT_REPEAT,
        })
        .collect();
    s.run(Command::Clip(clip::Command::NotesAdd { clip: c, notes }));
}

fn scattered(seed: f64, chance: f64) -> Session {
    let (mut s, fx, track) = session();
    set(&mut s, &fx, "mode", 1.0);
    set(&mut s, &fx, "chance", chance);
    set(&mut s, &fx, "choices", 4.0);
    set(&mut s, &fx, "interval", 1.0);
    set(&mut s, &fx, "direction", 2.0);
    set(&mut s, &fx, "seed", seed);
    arpeggio(&mut s, &track);
    s.run(Command::Transport(transport::Command::Play {}));
    s.render_secs(4.0);
    s.run(Command::Transport(transport::Command::Stop {}));
    s.render_secs(1.0);
    s
}

fn hz(key: u8) -> f32 {
    440.0 * 2f32.powf((f32::from(key) - 69.0) / 12.0)
}

/// The strongest bin within a quarter tone of `key`, over `window`.
fn level_at(r: &Render, window: std::ops::Range<f32>, key: u8) -> f32 {
    let f = hz(key);
    r.spectrum(window).peak_in(f * 0.97..f * 1.03)
}

#[test]
fn an_arpeggio_through_scatter_moves_within_f_minor_alike_on_both_targets_and_at_equal_seed() {
    let s = scattered(11.0, 50.0);
    let r = parity(s.scenario());
    r.assert_clean();
    r.assert_no_denormals();
    r.assert_audible(0.1..3.9, 0.01);
    // A second render at the same seed is the same, sample for sample.
    let again = render_native(scattered(11.0, 50.0).scenario());
    again.assert_close(&r, 0.0);
    // Another seed moves other notes.
    let other = render_native(scattered(12.0, 50.0).scenario());
    let diff: f32 = (0..40)
        .map(|i| {
            let w = i as f32 * 0.1..i as f32 * 0.1 + 0.1;
            (other.rms_in(w.clone()) - r.rms_in(w)).abs()
        })
        .sum();
    assert!(diff > 1e-3, "another seed, another melody: {diff}");
    // Moved notes are in F minor: against the arpeggio as written (chance 0), the keys of the
    // scale it does not play gain level, the keys off the scale do not.
    let written = render_native(scattered(11.0, 0.0).scenario());
    let whole = 0.0..4.0;
    let gain = |key: u8| level_at(&r, whole.clone(), key) / level_at(&written, whole.clone(), key);
    // The keys of the scale one or two degrees from the arpeggio's notes, never written (B♭3,
    // D♭4, E♭4, G4, B♭4, D♭5, G5, B♭5): at least two of them now sound (a note there comes out
    // more than 5 times above the written render's leak).
    let new_keys = [58u8, 61, 63, 67, 70, 73, 79, 82]
        .into_iter()
        .filter(|&k| gain(k) > 5.0)
        .count();
    assert!(
        new_keys >= 2,
        "moved notes on keys of the scale: {new_keys}"
    );
    // Off the scale, no key gains as a played note does: what rises is the leak of a
    // neighbour in the scale that now plays more (under 3 times).
    const F_MINOR: [u8; 7] = [5, 7, 8, 10, 0, 1, 3];
    for key in (52u8..90).filter(|k| !F_MINOR.contains(&(k % 12))) {
        assert!(gain(key) < 3.0, "key {key} off the scale: {}", gain(key));
    }
    // Nothing sounds after the stop and the release.
    assert!(r.rms_in(4.4..5.0) < 1e-4, "tail {}", r.rms_in(4.4..5.0));
    r.write_wav("scatter-arpeggio");
}

fn note_on(s: &mut Session, track: &TrackId, note: u8) {
    s.run(Command::Live(live::Command::NoteOn {
        track: track.clone(),
        note,
        velocity: 100,
    }));
}

fn note_off(s: &mut Session, track: &TrackId, note: u8) {
    s.run(Command::Live(live::Command::NoteOff {
        track: track.clone(),
        note,
    }));
}

#[test]
fn a_note_moved_then_bypassed_removed_changed_or_stopped_leaves_nothing_sounding() {
    for case in ["bypass", "remove", "settings", "unbypass", "stop"] {
        let (mut s, fx, track) = session();
        set(&mut s, &fx, "chance", 100.0);
        set(&mut s, &fx, "choices", 1.0);
        set(&mut s, &fx, "interval", 7.0);
        set(&mut s, &fx, "direction", 0.0);
        s.run(Command::Transport(transport::Command::Play {}));
        if case == "unbypass" {
            s.run(Command::Device(device::Command::SetBypass {
                device: fx.clone(),
                bypass: true,
            }));
        }
        note_on(&mut s, &track, 65);
        s.render_secs(0.3);
        match case {
            "bypass" => {
                s.run(Command::Device(device::Command::SetBypass {
                    device: fx.clone(),
                    bypass: true,
                }));
            }
            "unbypass" => {
                s.run(Command::Device(device::Command::SetBypass {
                    device: fx.clone(),
                    bypass: false,
                }));
            }
            "remove" => {
                s.run(Command::Device(device::Command::Remove {
                    device: fx.clone(),
                }));
            }
            "stop" => {
                s.run(Command::Transport(transport::Command::Stop {}));
            }
            _ => {
                for (param, value) in [
                    ("interval", 3.0),
                    ("direction", 1.0),
                    ("mode", 1.0),
                    ("chance", 0.0),
                    ("seed", 99.0),
                ] {
                    set(&mut s, &fx, param, value);
                }
            }
        }
        s.render_secs(0.3);
        note_off(&mut s, &track, 65);
        s.render_secs(1.0);
        let r = render_native(s.scenario());
        r.assert_clean();
        assert!(r.rms_in(0.05..0.3) > 1e-3, "{case}: the note sounded");
        assert!(
            r.rms_in(1.2..1.6) < 1e-4,
            "{case}: still sounding {}",
            r.rms_in(1.2..1.6)
        );
    }
}

/// The moved note sounds at its new pitch, the received one does not: one note, a fifth up.
#[test]
fn a_moved_note_sounds_at_its_new_pitch() {
    let (mut s, fx, track) = session();
    set(&mut s, &fx, "chance", 100.0);
    set(&mut s, &fx, "choices", 1.0);
    set(&mut s, &fx, "interval", 7.0);
    set(&mut s, &fx, "direction", 0.0);
    s.run(Command::Transport(transport::Command::Play {}));
    note_on(&mut s, &track, 60);
    s.render_secs(0.5);
    note_off(&mut s, &track, 60);
    s.render_secs(0.5);
    let r = parity(s.scenario());
    let window = 0.1..0.45;
    let g = level_at(&r, window.clone(), 67);
    let c = level_at(&r, window, 60);
    assert!(g > 10.0 * c, "g {g}, c {c}");
}

/// Criterion 2 on the WebAssembly build: 1 000 notes of a clip through Scatter at 50 %, with 24
/// choices a semitone apart around them, render alike natively and in WASM (the parity's
/// tolerance is far below a note moved by a semitone, so every one of the 1 000 draws is the
/// same on both targets); the share moved is counted natively, by the device, in
/// `crates/engine/tests/it/scatter.rs`.
#[test]
fn a_thousand_notes_through_scatter_draw_alike_natively_and_in_wasm() {
    let (mut s, fx, track) = session();
    set(&mut s, &fx, "chance", 50.0);
    set(&mut s, &fx, "choices", 24.0);
    set(&mut s, &fx, "interval", 1.0);
    set(&mut s, &fx, "direction", 2.0);
    set(&mut s, &fx, "seed", 691.0);
    s.run(Command::Transport(transport::Command::SetTempo {
        bpm: 240.0,
        placements: None,
    }));
    // 1 000 notes, a 64th note apart (60 ticks: 15.6 ms at 240 BPM), C3 to B5 in turn.
    let Output::Clip(clip::Output::Create { clip: c, .. }) =
        s.exec(Command::Clip(clip::Command::Create {
            track: track.clone(),
            at: clip::ClipTarget::Arrangement { start: Tick(0) },
            length: Some(Tick(60_000)),
            name: None,
        }))
    else {
        panic!("a clip")
    };
    let notes = (0..1000i64)
        .map(|i| Note {
            start: Tick(i * 60),
            pitch: 48 + (i % 36) as u8,
            length: Tick(40),
            velocity: 100,
            cents: 0,
            chance: Note::DEFAULT_CHANCE,
            repeat: Note::DEFAULT_REPEAT,
        })
        .collect();
    s.run(Command::Clip(clip::Command::NotesAdd { clip: c, notes }));
    s.run(Command::Transport(transport::Command::Play {}));
    s.render_secs(15.8);
    let r = parity(s.scenario());
    r.assert_clean();
    r.assert_audible(0.1..15.5, 0.005);
}
