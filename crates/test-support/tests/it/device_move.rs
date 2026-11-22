//! #800 through the command layer and the engine: a device inserted, removed or moved in
//! front of another moves it to its new slot without building it again, so an echo's or a
//! reverb's tail and a compressor's envelope go on. Each render is compared with the same
//! session without the edit (the tail and the envelope must be the same) and with the old
//! behaviour, which a removal and its undo still give (the device built again at its place:
//! its tail restarts). The renders are the same natively and in WASM, and are written to
//! `.qa/renders/device-move-*.wav` for the ear.

use commands::testing::{context, fixture};
use commands::{Command, Output, device, history, live, track};
use project::{DeviceId, TrackId, TrackKind};
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

/// The largest sample-to-sample move of the left channel over `window` (seconds).
fn largest_step(r: &Render, window: std::ops::Range<f32>) -> f32 {
    let (from, to) = (
        (window.start * test_support::SAMPLE_RATE) as usize,
        (window.end * test_support::SAMPLE_RATE) as usize,
    );
    r.left[from..to]
        .windows(2)
        .fold(0f32, |m, w| m.max((w[1] - w[0]).abs()))
}

/// #838: a device with a latency line (a Compressor, whatever its look-ahead, or a Ceiling)
/// removed in front of a ringing reverb, on the synth's track or on the master (`track`
/// `None`). The reverb moves into its slot behind it (on a track the removed device holds
/// the slot's timing until the slot switches, #761; on the master its line crossfades to
/// the undelayed signal) and keeps its tail. With `longer`, a silent second track holds a
/// longer chain (two Ceilings), so that the tracks' latency never moves. Gives the render
/// without the edit, the one with it, and the control (the reverb built again at its place:
/// a removal and its undo).
fn line_removed_in_front_of_a_reverb(
    kind: &str,
    lookahead: Option<f64>,
    on_master: bool,
    longer: bool,
) -> (Render, Render, Render) {
    let ring = 0.3;
    let session = |edit: u8| {
        let (mut s, track) = plucked();
        if longer {
            let out = s.exec(Command::Track(track::Command::Create {
                kind: TrackKind::Midi,
                instrument: Some("faulk.drums".into()),
                name: None,
                color: None,
            }));
            let Output::Track(track::Output::Create { track: other, .. }) = out else {
                panic!("{out:?}")
            };
            insert(&mut s, Some(&other), "faulk.ceiling", 0);
            insert(&mut s, Some(&other), "faulk.ceiling", 1);
        }
        let chain = (!on_master).then_some(&track);
        let lined = insert(&mut s, chain, kind, 0);
        if let Some(seconds) = lookahead {
            set(&mut s, &lined, "lookahead", seconds);
        }
        let reverb = insert(&mut s, chain, "faulk.reverb", 1);
        set(&mut s, &reverb, "size", 0.9);
        tail(&mut s, &track, ring, |s| match edit {
            1 => remove(s, &lined),
            2 => {
                remove(s, &reverb);
                undo(s);
            }
            _ => {}
        });
        s
    };
    (
        parity(session(0).scenario()),
        parity(session(1).scenario()),
        parity(session(2).scenario()),
    )
}

#[test]
fn a_device_with_a_line_removed_in_front_of_a_ringing_reverb_keeps_its_tail() {
    let at = edit_at(0.3);
    for (kind, lookahead, on_master, longer) in [
        ("faulk.compressor", None, false, false),
        ("faulk.compressor", Some(0.002), false, false),
        ("faulk.compressor", Some(0.002), false, true),
        ("faulk.ceiling", None, false, false),
        ("faulk.ceiling", None, false, true),
        ("faulk.compressor", None, true, false),
        ("faulk.compressor", Some(0.002), true, false),
        ("faulk.ceiling", None, true, false),
    ] {
        let name = format!(
            "{kind} (look-ahead {lookahead:?}) on the {}{}",
            if on_master { "master" } else { "track" },
            if longer {
                ", a longer track beside"
            } else {
                ""
            }
        );
        let (untouched, moved, rebuilt) =
            line_removed_in_front_of_a_reverb(kind, lookahead, on_master, longer);
        moved.assert_clean();
        if lookahead.is_none() && kind == "faulk.compressor" {
            moved.write_wav(if on_master {
                "device-move-compressor-reverb-master"
            } else {
                "device-move-compressor-reverb"
            });
        }
        let (kept, got, lost) = (
            rms_after(&untouched, at + 0.05),
            rms_after(&moved, at + 0.05),
            rms_after(&rebuilt, at + 0.05),
        );
        let (diff, diff_at) = moved.max_difference(&untouched);
        // Through the edit and the switch (or the master's crossfade) after it.
        let window = at - 0.01..at + 0.1;
        let (step, reference) = (
            largest_step(&moved, window.clone()),
            largest_step(&untouched, window),
        );
        println!(
            "{name}: reverb tail rms {kept:.5}, moved {got:.5}, rebuilt {lost:.5}, max \
             difference {diff:e} at {diff_at}, largest step {step:.5} (untouched {reference:.5})"
        );
        assert!(kept > 0.002, "{name}: the reverb rings: {kept}");
        // Where the tracks' latency stays (a longer track beside, a look-ahead of 0, the
        // master's chain, which no compensation follows), the render is the one without the
        // edit; elsewhere the track's delay moves with `T` once the slot switched (#761).
        if longer || on_master || lookahead.is_none() && kind == "faulk.compressor" {
            assert!(diff < 1e-6, "{name}: a difference of {diff}");
        }
        assert!(
            (got / kept - 1.0).abs() < 0.02,
            "{name}: the tail moved with the reverb: {got} against {kept}"
        );
        assert!(
            step <= reference * 1.05 + 1e-4,
            "{name}: a step of {step} against {reference}"
        );
        assert!(
            lost < 0.1 * kept,
            "{name}: the control: a rebuilt reverb starts silent ({lost})"
        );
    }
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
fn moving_an_effect_in_front_of_a_ringing_echo_keeps_the_echo() {
    // Synth, echo, filter (wide open). The filter moves in front of the echo: a swap of
    // neighbours, where the one going toward the head (the filter) is built again at its new
    // place and the other (the echo) moves with its tail. The control builds the echo again
    // (removed, then the undo).
    let ring = 0.5;
    let session = |edit: u8| {
        let (mut s, track) = plucked();
        let echo = insert(&mut s, Some(&track), "faulk.echo", 0);
        let filter = insert(&mut s, Some(&track), "faulk.filter", 1);
        set(&mut s, &filter, "cutoff", 20_000.0);
        tail(&mut s, &track, ring, |s| match edit {
            1 => {
                s.run(Command::Device(device::Command::Move {
                    device: filter.clone(),
                    index: 0,
                }));
            }
            2 => {
                remove(s, &echo);
                undo(s);
            }
            _ => {}
        });
        s
    };
    let untouched = parity(session(0).scenario());
    let moved = parity(session(1).scenario());
    let rebuilt = parity(session(2).scenario());
    moved.assert_clean();
    moved.write_wav("device-move-reorder");
    // After the crossfades of the edit.
    let at = edit_at(ring) + 0.05;
    let (kept, got, lost) = (
        rms_after(&untouched, at),
        rms_after(&moved, at),
        rms_after(&rebuilt, at),
    );
    println!("reorder tail rms {kept:.5}, moved {got:.5}, rebuilt {lost:.5}");
    assert!(kept > 0.002, "the echo rings: {kept}");
    // The filter, wide open, passes the echo's tail almost as is.
    assert!((got / kept - 1.0).abs() < 0.05, "{got} against {kept}");
    assert!(
        lost < 0.1 * kept,
        "the control: a rebuilt echo starts silent ({lost})"
    );
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

/// #838 on a sounding input, natively and in WASM: a held note through a Ceiling (below its
/// ceiling) and a reverb; the Ceiling is removed while the note sounds. On the track, with a
/// longer track beside (the tracks' latency stays), the reverb behind the held slot hears
/// the Ceiling's line, then catches up at the switch: the render is the one without the
/// edit. On the master, the reverb hears the line's crossfade to the undelayed signal: no
/// step larger than the untouched render's.
#[test]
fn a_ceiling_removed_in_front_of_a_reverb_on_a_held_note_keeps_its_input() {
    let session = |on_master: bool, edit: bool| {
        let f = fixture();
        let mut s = Session::from_state(f.state.clone(), context());
        let track = f.track.clone();
        let out = s.exec(Command::Track(track::Command::Create {
            kind: TrackKind::Midi,
            instrument: Some("faulk.drums".into()),
            name: None,
            color: None,
        }));
        let Output::Track(track::Output::Create { track: other, .. }) = out else {
            panic!("{out:?}")
        };
        insert(&mut s, Some(&other), "faulk.ceiling", 0);
        insert(&mut s, Some(&other), "faulk.ceiling", 1);
        let chain = (!on_master).then_some(&track);
        let ceiling = insert(&mut s, chain, "faulk.ceiling", 0);
        let reverb = insert(&mut s, chain, "faulk.reverb", 1);
        set(&mut s, &reverb, "size", 0.9);
        s.run(Command::Live(live::Command::NoteOn {
            track: track.clone(),
            note: 60,
            velocity: 80,
        }));
        s.render_secs(0.4);
        if edit {
            remove(&mut s, &ceiling);
        }
        s.render_secs(0.6);
        s.run(Command::Live(live::Command::AllNotesOff {}));
        s.render_secs(0.5);
        s
    };
    for on_master in [false, true] {
        let untouched = parity(session(on_master, false).scenario());
        let moved = parity(session(on_master, true).scenario());
        moved.assert_clean();
        let (diff, diff_at) = moved.max_difference(&untouched);
        let window = 0.38..0.6;
        let (step, reference) = (
            largest_step(&moved, window.clone()),
            largest_step(&untouched, window.clone()),
        );
        let level = untouched.rms_in(window);
        println!(
            "held note, Ceiling removed on the {}: rms {level:.5}, max difference {diff:e} at {diff_at}, largest step {step:.5} (untouched {reference:.5})",
            if on_master { "master" } else { "track" }
        );
        assert!(level > 0.01, "the note sounds: {level}");
        if on_master {
            assert!(
                step <= reference * 1.05 + 1e-4,
                "a step of {step} against {reference}"
            );
        } else {
            assert!(diff < 1e-6, "a difference of {diff}");
        }
    }
}
