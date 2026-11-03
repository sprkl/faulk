//! DV-6 through the command layer: a MIDI track made by `track.create` with the instrument
//! `faulk.drums` plays its notes in the engine, a parameter set by `device.set_param`
//! reaches the sound, and the render is the same natively and in WASM. (The device itself
//! is tested in `crates/engine/tests/it/drums.rs`.)

use commands::testing::{context, fixture};
use commands::{Command, Output, device, live, track};
use project::{DeviceId, TrackId, TrackKind};
use test_support::{Session, parity};

fn drums_session() -> (Session, TrackId, DeviceId) {
    let mut s = Session::from_state(fixture().state, context());
    let out = s.exec(Command::Track(track::Command::Create {
        kind: TrackKind::Midi,
        instrument: Some("faulk.drums".into()),
        name: None,
        color: None,
    }));
    let Output::Track(track::Output::Create { track, device }) = out else {
        panic!("{out:?}")
    };
    (s, track, device)
}

fn hit(s: &mut Session, track: &TrackId, note: u8) {
    s.run(Command::Live(live::Command::NoteOn {
        track: track.clone(),
        note,
        velocity: 110,
    }));
}

#[test]
fn a_drums_track_made_by_commands_plays_each_sound_on_its_note() {
    let (mut s, track, device) = drums_session();
    let notes = [36u8, 38, 39, 42, 46];
    for note in notes {
        hit(&mut s, &track, note);
        s.render_secs(0.4);
        // Silence the sound, then a gap to tell the hits apart.
        s.run(Command::Live(live::Command::AllNotesOff {}));
        s.render_secs(0.2);
    }
    // A note the drums do not play makes no sound (from 3.0 s).
    hit(&mut s, &track, 61);
    s.render_secs(0.3);
    // A parameter set by name reaches the sound: the kick at level 0 is silent (from about
    // 3.36 s: the level ramps for 10 ms).
    s.run(Command::Device(device::Command::SetParam {
        device,
        param: "kick_level".into(),
        value: 0.0,
    }))
    .render_secs(0.05);
    hit(&mut s, &track, 36);
    s.render_secs(0.4);
    let r = parity(s.scenario());
    r.assert_clean();
    for i in 0..notes.len() {
        let start = i as f32 * 0.6;
        r.assert_audible(start..start + 0.4, 0.02);
    }
    let kick = r.spectrum(0.05..0.3).dominant_hz();
    assert!(kick < 120.0, "the kick is low: {kick} Hz");
    r.assert_silent(3.0..3.3);
    r.assert_silent(3.37..r.seconds());
    r.write_wav("drums-through-commands");
}
