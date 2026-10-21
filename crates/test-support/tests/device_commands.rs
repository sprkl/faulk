//! DV-8 through the command layer and the engine: `track.set_instrument` swaps the
//! instrument in a running engine (no device left behind in the slot), `device.set_bypass`
//! silences it, and the undo of each brings the sound back. The render is the same
//! natively and in WASM. (The commands themselves are tested in
//! `crates/commands/tests/device.rs`.)

use commands::testing::{context, fixture};
use commands::{Command, Output, device, history, live, track};
use test_support::{Session, parity};

fn note(s: &mut Session, track: &project::TrackId, pitch: u8) {
    s.run(Command::Live(live::Command::NoteOn {
        track: track.clone(),
        note: pitch,
        velocity: 110,
    }));
}

fn release(s: &mut Session) {
    s.run(Command::Live(live::Command::AllNotesOff {}));
}

#[test]
fn swapping_the_instrument_changes_what_a_note_plays_and_undo_gives_the_synth_back() {
    let f = fixture();
    let mut s = Session::from_state(f.state.clone(), context());
    // The synth plays note 61 (the drums have no sound on it).
    note(&mut s, &f.track, 61);
    s.render_secs(0.3);
    release(&mut s);
    s.render_secs(1.0);
    // The drums replace it: the same note is now silent, the kick plays.
    let out = s.exec(Command::Track(track::Command::SetInstrument {
        track: f.track.clone(),
        instrument: "faulk.drums".into(),
    }));
    let Output::Track(track::Output::SetInstrument { device, previous }) = out else {
        panic!("{out:?}")
    };
    assert_eq!(previous, f.device);
    assert_ne!(device, f.device);
    note(&mut s, &f.track, 61);
    s.render_secs(0.3);
    release(&mut s);
    s.render_secs(0.5);
    note(&mut s, &f.track, 36);
    s.render_secs(0.3);
    release(&mut s);
    s.render_secs(0.5);
    // Undo: the synth is back, with the same note audible.
    s.run(Command::History(history::Command::Undo {}));
    note(&mut s, &f.track, 61);
    s.render_secs(0.3);
    release(&mut s);
    s.render_secs(0.5);
    let r = parity(s.scenario());
    r.assert_clean();
    // Phases, in seconds: synth 61 at 0 (to 1.3), drums 61 at 1.3, kick at 2.1, synth 61
    // again at 2.9.
    r.assert_audible(0.05..0.3, 0.02);
    r.assert_silent(1.35..1.6);
    r.assert_audible(2.15..2.4, 0.02);
    r.assert_audible(2.95..3.2, 0.02);
    r.write_wav("set-instrument-through-commands");
}

#[test]
fn a_bypassed_instrument_is_silent_and_undo_gives_it_back() {
    let f = fixture();
    let mut s = Session::from_state(f.state.clone(), context());
    s.run(Command::Device(device::Command::SetBypass {
        device: f.device.clone(),
        bypass: true,
    }));
    note(&mut s, &f.track, 60);
    s.render_secs(0.3);
    release(&mut s);
    s.render_secs(0.5);
    s.run(Command::History(history::Command::Undo {}));
    note(&mut s, &f.track, 60);
    s.render_secs(0.3);
    let r = parity(s.scenario());
    r.assert_clean();
    r.assert_silent(0.0..0.8);
    r.assert_audible(0.85..1.1, 0.02);
}
