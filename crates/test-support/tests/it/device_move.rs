//! #800 through the command layer and the engine: a device inserted, removed or moved in
//! front of another moves it to its new slot without building it again, so an echo's or a
//! reverb's tail and a compressor's envelope go on. Each render is compared with the same
//! session without the edit (the tail and the envelope must be the same) and with the old
//! behaviour, which a removal and its undo still give (the device built again at its place:
//! its tail restarts). The renders are the same natively and in WASM, and are written to
//! `.qa/renders/device-move-*.wav` for the ear.

use commands::testing::{context, fixture};
use commands::{Command, Output, device, history, live};
use project::{DeviceId, TrackId};
use test_support::{Render, Session, parity};

fn insert(s: &mut Session, track: Option<&TrackId>, kind: &str, index: u32) -> DeviceId {
    let out = s.exec(Command::Device(device::Command::Insert {
        track: track.cloned(),
        master: track.is_none(),
        kind: kind.into(),
        index,
    }));
    let Output::Device(device::Output::Insert { device }) = out else {
        panic!("{out:?}")
    };
    device
}

fn set(s: &mut Session, device: &DeviceId, param: &str, value: f64) {
    s.run(Command::Device(device::Command::SetParam {
        device: device.clone(),
        param: param.into(),
        value,
    }));
}

fn remove(s: &mut Session, device: &DeviceId) {
    s.run(Command::Device(device::Command::Remove {
        device: device.clone(),
    }));
}

fn undo(s: &mut Session) {
    s.run(Command::History(history::Command::Undo {}));
}

/// A session on the fixture's synth, set to a pluck (it releases in a few milliseconds).
fn plucked() -> (Session, TrackId) {
    let f = fixture();
    let mut s = Session::from_state(f.state.clone(), context());
    for (param, value) in [("attack", 0.002), ("release", 0.01)] {
        set(&mut s, &f.device, param, value);
    }
    (s, f.track.clone())
}

/// A pluck, then its tail rings for `ring` seconds before `edit`, then 2 s more.
fn tail(s: &mut Session, track: &TrackId, ring: f32, edit: impl FnOnce(&mut Session)) {
    s.run(Command::Live(live::Command::NoteOn {
        track: track.clone(),
        note: 60,
        velocity: 110,
    }));
    s.render_secs(0.1);
    s.run(Command::Live(live::Command::AllNotesOff {}));
    s.render_secs(ring);
    edit(s);
    s.render_secs(2.0);
}

/// The edit lands at this time in [`tail`]'s render (rounded up to a block by the session).
fn edit_at(ring: f32) -> f32 {
    0.1 + ring + 0.01
}

fn rms_after(r: &Render, at: f32) -> f32 {
    r.rms_in(at..at + 1.5)
}

#[test]
fn an_effect_inserted_in_front_of_a_ringing_echo_keeps_its_tail() {
    let ring = 0.5;
    let session = |edit: fn(&mut Session, &DeviceId, &TrackId)| {
        let (mut s, track) = plucked();
        let echo = insert(&mut s, Some(&track), "faulk.echo", 0);
        tail(&mut s, &track, ring, |s| edit(s, &echo, &track));
        s
    };
    let untouched = parity(session(|_, _, _| {}).scenario());
    let moved = parity(
        session(|s, _, track| {
            insert(s, Some(track), "faulk.filter", 0);
        })
        .scenario(),
    );
    let rebuilt = parity(
        session(|s, echo, _| {
            remove(s, echo);
            undo(s);
        })
        .scenario(),
    );
    moved.assert_clean();
    moved.write_wav("device-move-echo");
    let at = edit_at(ring);
    let (kept, lost) = (rms_after(&untouched, at), rms_after(&rebuilt, at));
    // The filter in front hears silence (the pluck is over): the echo's tail is the
    // untouched one, sample for sample.
    let (diff, _) = moved.max_difference(&untouched);
    println!(
        "echo tail rms {kept:.5}, moved {:.5}, rebuilt {lost:.5}, diff {diff:e}",
        rms_after(&moved, at)
    );
    assert!(kept > 0.002, "the echo rings: {kept}");
    assert!(diff < 1e-6, "the tail moved with the echo: {diff}");
    assert!(
        lost < 0.1 * kept,
        "the control: a rebuilt echo starts silent ({lost})"
    );
}

#[test]
fn an_effect_removed_in_front_of_a_ringing_reverb_keeps_its_tail() {
    let ring = 0.3;
    let session = |remove_it: bool| {
        let (mut s, track) = plucked();
        let filter = insert(&mut s, Some(&track), "faulk.filter", 0);
        set(&mut s, &filter, "mix", 0.0);
        let reverb = insert(&mut s, Some(&track), "faulk.reverb", 1);
        set(&mut s, &reverb, "size", 0.9);
        tail(&mut s, &track, ring, |s| {
            if remove_it {
                remove(s, &filter);
            } else {
                // The control: the reverb built again at its place.
                remove(s, &reverb);
                undo(s);
            }
        });
        s
    };
    let (mut untouched, track) = plucked();
    let filter = insert(&mut untouched, Some(&track), "faulk.filter", 0);
    set(&mut untouched, &filter, "mix", 0.0);
    let reverb = insert(&mut untouched, Some(&track), "faulk.reverb", 1);
    set(&mut untouched, &reverb, "size", 0.9);
    tail(&mut untouched, &track, ring, |_| {});
    let untouched = parity(untouched.scenario());
    let moved = parity(session(true).scenario());
    let rebuilt = parity(session(false).scenario());
    moved.assert_clean();
    moved.write_wav("device-move-reverb");
    let at = edit_at(ring);
    let (kept, lost) = (rms_after(&untouched, at), rms_after(&rebuilt, at));
    let (diff, _) = moved.max_difference(&untouched);
    println!(
        "reverb tail rms {kept:.5}, moved {:.5}, rebuilt {lost:.5}, diff {diff:e}",
        rms_after(&moved, at)
    );
    assert!(kept > 0.002, "the reverb rings: {kept}");
    assert!(diff < 1e-6, "the tail moved with the reverb: {diff}");
    assert!(
        lost < 0.1 * kept,
        "the control: a rebuilt reverb starts silent ({lost})"
    );
}

#[test]
fn a_compressor_moved_by_an_insert_in_front_keeps_its_envelope() {
    // A loud note pushes the compressor into gain reduction; it releases slowly. During the
    // short gap before the next note an effect is inserted in front of it (its mix at 0: it
    // passes the signal as is). The second note starts against the compressor's reduction,
    // as it does without the insert; a compressor built again starts with none (a jump).
    let session = |edit: u8| {
        let f = fixture();
        let mut s = Session::from_state(f.state.clone(), context());
        set(&mut s, &f.device, "attack", 0.001);
        set(&mut s, &f.device, "release", 0.01);
        let comp = insert(&mut s, Some(&f.track), "faulk.compressor", 0);
        for (param, value) in [
            ("threshold", -30.0),
            ("ratio", 8.0),
            ("attack", 0.005),
            ("release", 1.0),
        ] {
            set(&mut s, &comp, param, value);
        }
        let note = |s: &mut Session| {
            s.run(Command::Live(live::Command::NoteOn {
                track: f.track.clone(),
                note: 48,
                velocity: 127,
            }));
        };
        note(&mut s);
        s.render_secs(0.5);
        s.run(Command::Live(live::Command::AllNotesOff {}));
        s.render_secs(0.05);
        match edit {
            1 => {
                let filter = insert(&mut s, Some(&f.track), "faulk.filter", 0);
                s.render_blocks(1);
                set(&mut s, &filter, "mix", 0.0);
            }
            2 => {
                remove(&mut s, &comp);
                undo(&mut s);
                s.render_blocks(1);
            }
            _ => {
                s.render_blocks(1);
            }
        }
        s.render_secs(0.05);
        note(&mut s);
        s.render_secs(0.3);
        s
    };
    let untouched = parity(session(0).scenario());
    let moved = parity(session(1).scenario());
    let rebuilt = parity(session(2).scenario());
    moved.assert_clean();
    moved.write_wav("device-move-compressor");
    // The second note's first 20 ms.
    let onset = 0.5 + 0.05 + 0.05 + 128.0 / test_support::SAMPLE_RATE;
    let window = onset..onset + 0.02;
    let (kept, moved_peak, lost) = (
        untouched.peak_in(window.clone()),
        moved.peak_in(window.clone()),
        rebuilt.peak_in(window),
    );
    println!("compressor onset peak {kept:.5}, moved {moved_peak:.5}, rebuilt {lost:.5}");
    assert!(
        (moved_peak / kept - 1.0).abs() < 0.01,
        "the envelope moved with the compressor: {moved_peak} against {kept}"
    );
    assert!(
        lost > 1.3 * kept,
        "the control: a new compressor lets the onset through ({lost})"
    );
}

#[test]
fn moving_an_effect_across_a_ringing_echo_keeps_the_echo() {
    // Synth, echo, filter (wide open). The filter moves in front of the echo, then the echo
    // back in front: the echo keeps its instance (its tail), the filter is built again at its
    // new place each time (a reorder builds again the device that goes furthest).
    let ring = 0.5;
    let session = |reorder: bool| {
        let (mut s, track) = plucked();
        let echo = insert(&mut s, Some(&track), "faulk.echo", 0);
        let filter = insert(&mut s, Some(&track), "faulk.filter", 1);
        set(&mut s, &filter, "cutoff", 20_000.0);
        tail(&mut s, &track, ring, |s| {
            if reorder {
                s.run(Command::Device(device::Command::Move {
                    device: filter.clone(),
                    index: 0,
                }));
                s.render_secs(0.5);
                s.run(Command::Device(device::Command::Move {
                    device: echo.clone(),
                    index: 0,
                }));
            } else {
                s.render_secs(0.5);
            }
        });
        s
    };
    let untouched = parity(session(false).scenario());
    let moved = parity(session(true).scenario());
    moved.assert_clean();
    moved.write_wav("device-move-reorder");
    let at = edit_at(ring);
    // The tail goes on: the filter, wide open, passes the echo's tail almost as is.
    let (kept, got) = (rms_after(&untouched, at), rms_after(&moved, at));
    println!("reorder tail rms {kept:.5}, moved {got:.5}");
    assert!(kept > 0.002, "the echo rings: {kept}");
    assert!((got / kept - 1.0).abs() < 0.15, "{got} against {kept}");
}

#[test]
fn an_effect_inserted_at_the_head_of_the_master_keeps_the_master_echos_tail() {
    let ring = 0.5;
    let session = |insert_it: bool| {
        let (mut s, track) = plucked();
        insert(&mut s, None, "faulk.echo", 0);
        tail(&mut s, &track, ring, |s| {
            if insert_it {
                insert(s, None, "faulk.filter", 0);
            }
        });
        s
    };
    let untouched = parity(session(false).scenario());
    let moved = parity(session(true).scenario());
    moved.assert_clean();
    let (diff, _) = moved.max_difference(&untouched);
    let kept = rms_after(&untouched, edit_at(ring));
    println!("master echo tail rms {kept:.5}, diff {diff:e}");
    assert!(kept > 0.002, "the master echo rings: {kept}");
    assert!(diff < 1e-6, "the tail moved with the master echo: {diff}");
}
