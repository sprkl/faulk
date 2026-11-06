//! Stack (#690) through the command layer and the engine: `project.set_scale` and
//! `device.insert` put Stack before the synth of a track, a chord progression played as single
//! notes comes out as strummed triads of the scale (tension and chance on), the same natively
//! and in WASM, and is written to `.qa/renders/stack-progression.wav` for the ear. A chord held while
//! Stack is bypassed, removed or reset leaves nothing sounding. (The device itself is tested in
//! `crates/engine/tests/it/stack.rs`.)

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

/// A session on the fixture's synth track, in A minor, with Stack before the synth: scale mode,
/// +2 and +4 degrees (the triads of the scale) and, with `strum`, an octave above at 60 %
/// chance, a 60 ms strum with a little tension.
fn session(strum: bool) -> (Session, DeviceId, TrackId) {
    let f = fixture();
    let mut s = Session::from_state(f.state.clone(), context());
    s.run(Command::Project(p::Command::SetScale {
        root: 9,
        mode: ScaleKind::NaturalMinor,
    }));
    let out = s.exec(Command::Device(device::Command::Insert {
        track: Some(f.track.clone()),
        master: false,
        kind: "faulk.stack".into(),
        index: 0,
    }));
    let Output::Device(device::Output::Insert { device: fx }) = out else {
        panic!("{out:?}")
    };
    set(&mut s, &fx, "mode", 1.0);
    set(&mut s, &fx, "voice1_interval", 2.0);
    set(&mut s, &fx, "voice2", 1.0);
    set(&mut s, &fx, "voice2_interval", 4.0);
    if strum {
        set(&mut s, &fx, "voice3", 1.0);
        set(&mut s, &fx, "voice3_interval", 7.0);
        set(&mut s, &fx, "voice3_chance", 60.0);
        set(&mut s, &fx, "voice3_velocity", 70.0);
        set(&mut s, &fx, "strum", 0.06);
        set(&mut s, &fx, "tension", 30.0);
        set(&mut s, &fx, "crescendo", -20.0);
    }
    set(&mut s, &f.device, "release", 0.25);
    set(&mut s, &f.device, "gain", 0.5);
    s.run(Command::Transport(transport::Command::Play {}));
    (s, fx, f.track)
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

fn hz(key: u8) -> f32 {
    440.0 * 2f32.powf((f32::from(key) - 69.0) / 12.0)
}

/// The strongest bin within a quarter tone of `key`, over `window`.
fn level_at(r: &Render, window: std::ops::Range<f32>, key: u8) -> f32 {
    let f = hz(key);
    r.spectrum(window).peak_in(f * 0.97..f * 1.03)
}

#[test]
fn a_progression_of_single_notes_comes_out_as_strummed_triads_of_the_scale_on_both_targets() {
    let (mut s, _, track) = session(true);
    // Am, F, C, G in A minor: A3, F3, C4, G3, one second each.
    for root in [57u8, 53, 60, 55] {
        note_on(&mut s, &track, root);
        s.render_secs(0.9);
        note_off(&mut s, &track, root);
        s.render_secs(0.1);
    }
    s.render_secs(1.5);
    let r = parity(s.scenario());
    r.assert_clean();
    r.assert_no_denormals();
    r.assert_audible(0.2..3.9, 0.01);
    // The first chord, A C E: the third and fifth sound with the root; B (not in the triad)
    // stays far below them.
    let window = 0.3..0.85;
    let a = level_at(&r, window.clone(), 57);
    let c = level_at(&r, window.clone(), 60);
    let e = level_at(&r, window.clone(), 64);
    let b = level_at(&r, window.clone(), 59);
    assert!(c > 0.2 * a && e > 0.2 * a, "a {a}, c {c}, e {e}");
    assert!(b < 0.1 * c, "b {b}, c {c}");
    // The second chord, F A C: A in it, G sharp not.
    let window = 1.3..1.85;
    assert!(level_at(&r, window.clone(), 57) > 10.0 * level_at(&r, window, 56));
    // Nothing sounds once the last note's release is over.
    assert!(r.rms_in(4.8..5.5) < 1e-4, "tail {}", r.rms_in(4.8..5.5));
    r.write_wav("stack-progression");
}

#[test]
fn a_chord_released_while_stack_is_bypassed_does_not_come_back_when_it_is_switched_on() {
    // The strum's later notes were pending when Stack was bypassed; the note-off reached only
    // the instrument. Switched on again, Stack starts none of them: the engine resets it and
    // the switch's release reaches it first (either alone would do; the reset is the engine's
    // own test, `graph::a_midi_effect_is_cleared_when_it_is_switched_on_or_off`).
    let (mut s, fx, track) = session(true);
    set(&mut s, &fx, "strum", 0.4);
    note_on(&mut s, &track, 57);
    s.render_secs(0.05);
    let bypass = |bypass| {
        Command::Device(device::Command::SetBypass {
            device: fx.clone(),
            bypass,
        })
    };
    s.run(bypass(true));
    s.render_secs(0.05);
    note_off(&mut s, &track, 57);
    s.render_secs(0.05);
    s.run(bypass(false));
    s.render_secs(2.0);
    let r = render_native(s.scenario());
    assert!(
        r.rms_in(1.5..2.15) < 1e-4,
        "a note came back: {}",
        r.rms_in(1.5..2.15)
    );
}

#[test]
fn a_chord_held_while_stack_is_bypassed_removed_or_changed_leaves_nothing_sounding() {
    for case in ["bypass", "remove", "settings"] {
        let (mut s, fx, track) = session(true);
        set(&mut s, &fx, "strum", 0.4);
        note_on(&mut s, &track, 57);
        // Mid-strum: some notes started, others pending.
        s.render_secs(0.15);
        match case {
            "bypass" => {
                s.run(Command::Device(device::Command::SetBypass {
                    device: fx.clone(),
                    bypass: true,
                }));
            }
            "remove" => {
                s.run(Command::Device(device::Command::Remove {
                    device: fx.clone(),
                }));
            }
            _ => {
                for (param, value) in [
                    ("voice1", 0.0),
                    ("voice2_interval", 9.0),
                    ("mode", 0.0),
                    ("strum", 0.0),
                    ("direction", 1.0),
                ] {
                    set(&mut s, &fx, param, value);
                }
            }
        }
        s.render_secs(0.5);
        note_off(&mut s, &track, 57);
        s.render_secs(1.5);
        let r = render_native(s.scenario());
        r.assert_clean();
        assert!(
            r.rms_in(1.6..2.15) < 1e-4,
            "{case}: still sounding {}",
            r.rms_in(1.6..2.15)
        );
    }
}

/// #219, heard: a MIDI effect inserted, then removed, while a chord rings through a reverb does
/// not build the synth or the reverb again. Its insertion ends the held note as a note-off at
/// the same block would (the instrument's release, the reverb's tail going on), so the render
/// is the one of that note-off, sample for sample; its removal touches nothing that sounds.
#[test]
fn a_midi_effect_inserted_while_playing_cuts_neither_the_voices_nor_the_tails() {
    let render = |insert: bool| {
        let f = fixture();
        let mut s = Session::from_state(f.state.clone(), context());
        s.run(Command::Device(device::Command::Insert {
            track: Some(f.track.clone()),
            master: false,
            kind: "faulk.reverb".into(),
            index: 0,
        }));
        set(&mut s, &f.device, "release", 0.4);
        s.run(Command::Transport(transport::Command::Play {}));
        note_on(&mut s, &f.track, 57);
        s.render_secs(0.5);
        if insert {
            let out = s.exec(Command::Device(device::Command::Insert {
                track: Some(f.track.clone()),
                master: false,
                kind: "faulk.stack".into(),
                index: 0,
            }));
            let Output::Device(device::Output::Insert { device: fx }) = out else {
                panic!("{out:?}")
            };
            s.render_secs(0.5);
            s.run(Command::Device(device::Command::Remove { device: fx }));
        } else {
            note_off(&mut s, &f.track, 57);
            s.render_secs(0.5);
        }
        s.render_secs(1.0);
        render_native(s.scenario())
    };
    let inserted = render(true);
    let released = render(false);
    inserted.assert_clean();
    assert!(inserted.rms_in(0.6..1.0) > 1e-3, "the tail rings on");
    inserted.assert_close(&released, 1e-6);
}

/// A clip's note through Stack (the schedule, not the live input), with the 200 ms strum of
/// three notes at tension 0 (the criterion's case): the same natively and in WASM; the
/// transport stopped mid-strum ends the chord and starts nothing after.
#[test]
fn a_clip_note_strummed_at_tension_0_renders_alike_on_both_targets_and_a_stop_ends_it() {
    let f = fixture();
    let mut s = Session::from_state(f.state.clone(), context());
    let out = s.exec(Command::Device(device::Command::Insert {
        track: Some(f.track.clone()),
        master: false,
        kind: "faulk.stack".into(),
        index: 0,
    }));
    let Output::Device(device::Output::Insert { device: fx }) = out else {
        panic!("{out:?}")
    };
    set(&mut s, &fx, "voice1_interval", 4.0);
    set(&mut s, &fx, "voice2", 1.0);
    set(&mut s, &fx, "voice2_interval", 7.0);
    set(&mut s, &fx, "strum", 0.2);
    set(&mut s, &f.device, "release", 0.2);
    let Output::Clip(clip::Output::Create { clip: c, .. }) =
        s.exec(Command::Clip(clip::Command::Create {
            track: f.track.clone(),
            at: clip::ClipTarget::Arrangement { start: Tick(0) },
            length: Some(Tick(7680)),
            name: None,
        }))
    else {
        panic!("a clip")
    };
    s.run(Command::Clip(clip::Command::NotesAdd {
        clip: c,
        notes: vec![Note {
            start: Tick(0),
            pitch: 60,
            length: Tick(7680),
            velocity: 110,
            cents: 0,
            chance: Note::DEFAULT_CHANCE,
            repeat: Note::DEFAULT_REPEAT,
        }],
    }));
    s.run(Command::Transport(transport::Command::Play {}));
    s.render_secs(0.5);
    // Mid-strum of a second pass would be after 0.5 s: stop now, with the chord sounding.
    s.run(Command::Transport(transport::Command::Stop {}));
    s.render_secs(1.5);
    let r = parity(s.scenario());
    r.assert_clean();
    r.assert_audible(0.05..0.45, 0.01);
    // E4 comes in at 100 ms and G4 at 200 ms: the level rises at each.
    assert!(r.rms_in(0.12..0.19) > 1.2 * r.rms_in(0.02..0.09));
    assert!(r.rms_in(0.22..0.4) > 1.1 * r.rms_in(0.12..0.19));
    assert!(
        r.rms_in(1.2..2.0) < 1e-4,
        "still sounding {}",
        r.rms_in(1.2..2.0)
    );
}

/// A key held on the live input keeps its chord through a stop of the transport (the person
/// still holds it: the stop ends the clips' notes, not the hands'), the strum included; its
/// release ends the whole chord.
#[test]
fn a_held_key_keeps_its_strummed_chord_through_a_stop_and_its_release_ends_it() {
    let (mut s, fx, track) = session(false);
    set(&mut s, &fx, "strum", 0.4);
    note_on(&mut s, &track, 57);
    s.render_secs(0.05);
    s.run(Command::Transport(transport::Command::Stop {}));
    s.render_secs(0.6);
    note_off(&mut s, &track, 57);
    s.render_secs(1.5);
    let r = render_native(s.scenario());
    r.assert_clean();
    // The strum's last note (due at 400 ms) came after the stop.
    assert!(r.rms_in(0.45..0.6) > 1.1 * r.rms_in(0.06..0.12));
    assert!(
        r.rms_in(1.6..2.15) < 1e-4,
        "still sounding {}",
        r.rms_in(1.6..2.15)
    );
}
