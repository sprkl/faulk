//! `live.all_sound_off` (#331) through the command layer and the engine: a note with a long
//! release into an echo at its highest feedback is cut, on one track and on several, with
//! the transport playing. The render is the same natively and in WASM, and the first scenario
//! is written to `.qa/renders/all-sound-off.wav` for the ear. The sample-level measures
//! (-90 dBFS within 10 ms, steps, allocation detector) are `crates/engine/tests/it/all_sound_off.rs`.

use commands::testing::{context, fixture};
use commands::{Command, Output, clip, device, live, track, transport};
use project::{Note, Tick};
use test_support::{Session, parity};

/// Sets `param` of `device` to `value`.
fn set(s: &mut Session, device: &project::DeviceId, param: &str, value: f64) {
    s.run(Command::Device(device::Command::SetParam {
        device: device.clone(),
        param: param.into(),
        value,
    }));
}

/// An echo after the synth of the fixture's track: free time, the highest feedback, mostly wet.
fn echo_after(s: &mut Session, track: &project::TrackId) -> project::DeviceId {
    let out = s.exec(Command::Device(device::Command::Insert {
        track: Some(track.clone()),
        master: false,
        kind: "faulk.echo".into(),
        index: 0,
    }));
    let Output::Device(device::Output::Insert { device: echo }) = out else {
        panic!("{out:?}")
    };
    set(s, &echo, "mode", 0.0);
    set(s, &echo, "time", 0.25);
    set(s, &echo, "feedback", 0.95);
    set(s, &echo, "mix", 0.6);
    echo
}

fn cut(s: &mut Session) {
    s.run(Command::Live(live::Command::AllSoundOff {}));
}

#[test]
fn a_ringing_echo_and_a_long_release_are_cut_and_the_sound_comes_back() {
    let f = fixture();
    let mut s = Session::from_state(f.state.clone(), context());
    set(&mut s, &f.device, "release", 6.0);
    echo_after(&mut s, &f.track);
    s.run(Command::Live(live::Command::NoteOn {
        track: f.track.clone(),
        note: 57,
        velocity: 110,
    }));
    s.render_secs(0.5);
    s.run(Command::Live(live::Command::NoteOff {
        track: f.track.clone(),
        note: 57,
    }));
    s.render_secs(2.5);
    cut(&mut s); // at 3.0 s
    s.render_secs(2.0);
    s.run(Command::Live(live::Command::NoteOn {
        track: f.track.clone(),
        note: 60,
        velocity: 110,
    }));
    s.render_secs(1.0); // the new note, at 5.0 s

    let r = parity(s.scenario());
    r.assert_clean();
    // Still ringing just before the cut (the release, and the echo of the echo).
    r.assert_audible(2.9..3.0, 0.02);
    // Silent 10 ms after the cut and until the new note: no tail comes back, even though the
    // delay line was far from empty and the release had seconds to run.
    r.assert_silent(3.02..5.0);
    r.assert_audible(5.05..5.4, 0.05);
    r.write_wav("all-sound-off");
}

#[test]
fn a_cut_while_the_song_plays_silences_every_track_and_the_next_notes_still_sound() {
    let f = fixture();
    let mut s = Session::from_state(f.state.clone(), context());
    set(&mut s, &f.device, "release", 6.0);
    echo_after(&mut s, &f.track);
    // A second track with the same chain, played by hand.
    let Output::Track(track::Output::Duplicate { track: other }) =
        s.exec(Command::Track(track::Command::Duplicate {
            track: f.track.clone(),
            name: Some("Other".into()),
        }))
    else {
        panic!("a track")
    };
    // The first track plays a clip: a note at 0, another at bar 2 (2 s at 120 BPM).
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
    let note = |start, pitch| Note {
        start: Tick(start),
        pitch,
        length: Tick(600),
        velocity: 110,
        cents: 0,
        chance: Note::DEFAULT_CHANCE,
        repeat: Note::DEFAULT_REPEAT,
    };
    s.run(Command::Clip(clip::Command::NotesAdd {
        clip: c,
        notes: vec![note(0, 52), note(3840, 55)],
    }));
    s.run(Command::Transport(transport::Command::Play {}));
    s.run(Command::Live(live::Command::NoteOn {
        track: other,
        note: 64,
        velocity: 110,
    }));
    s.render_secs(1.0);
    cut(&mut s); // at 1.0 s, the transport keeps playing
    s.render_secs(2.0);

    let r = parity(s.scenario());
    r.assert_clean();
    r.assert_audible(0.9..1.0, 0.02);
    // Both tracks silent between the cut and the clip's second note at 2.0 s.
    r.assert_silent(1.02..1.99);
    r.assert_audible(2.05..2.4, 0.05);
}
