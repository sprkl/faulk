//! #159 through the command layer and the engine: `device.insert` puts Ceiling at the end of
//! the master's chain, `device.set_param` (by name, and undone through the history) and
//! `device.set_bypass` move it, under a drum loop pushed by 18 dB; the render is the same
//! natively and in WASM. The render, the loop dry (bypassed), pushed against a -3 dB
//! ceiling, then against a -6 dB ceiling with the true-peak detection, is written to
//! `.qa/renders/ceiling.wav` for the ear. (The device itself is tested in
//! `crates/engine/tests/it/ceiling.rs`.)

use commands::testing::{context, fixture};
use commands::{Command, Output, device, history, live, track};
use project::{DeviceId, TrackId, TrackKind};
use test_support::{Session, parity};

/// One step of the loop, in blocks of 128 frames: about 0.2507 s (an eighth note at 120 BPM).
const STEP_BLOCKS: u32 = 94;
/// A section: two bars of eight steps.
const SECTION_STEPS: u32 = 16;

fn hit(s: &mut Session, track: &TrackId, note: u8, velocity: u8) {
    s.run(Command::Live(live::Command::NoteOn {
        track: track.clone(),
        note,
        velocity,
    }));
}

/// Two bars of drums: kick (36), snare (38) and closed hat (42), with accents.
fn drum_loop(s: &mut Session, track: &TrackId) {
    const BAR: [(u8, u8, u8); 8] = [
        (120, 0, 90),
        (0, 0, 50),
        (0, 115, 90),
        (0, 0, 50),
        (110, 0, 90),
        (95, 0, 50),
        (0, 120, 90),
        (0, 0, 45),
    ];
    for _ in 0..2 {
        for (kick, snare, hat) in BAR {
            if kick > 0 {
                hit(s, track, 36, kick);
            }
            if snare > 0 {
                hit(s, track, 38, snare);
            }
            hit(s, track, 42, hat);
            s.render_blocks(STEP_BLOCKS);
        }
    }
}

fn set(s: &mut Session, fx: &DeviceId, param: &str, value: f64) {
    s.run(Command::Device(device::Command::SetParam {
        device: fx.clone(),
        param: param.into(),
        value,
    }));
}

fn bypass(s: &mut Session, fx: &DeviceId, bypass: bool) {
    s.run(Command::Device(device::Command::SetBypass {
        device: fx.clone(),
        bypass,
    }));
}

fn param(s: &Session, fx: &DeviceId, key: &str) -> Option<f64> {
    s.state().project.master_devices[fx]
        .params
        .get(key)
        .copied()
}

#[test]
fn ceiling_holds_a_drum_loop_pushed_18_db_on_the_master() {
    let mut s = Session::from_state(fixture().state, context());
    let out = s.exec(Command::Track(track::Command::Create {
        kind: TrackKind::Midi,
        instrument: Some("faulk.drums".into()),
        name: None,
        color: None,
    }));
    let Output::Track(track::Output::Create { track, .. }) = out else {
        panic!("{out:?}")
    };
    let out = s.exec(Command::Device(device::Command::Insert {
        track: None,
        master: true,
        kind: "faulk.ceiling".into(),
        index: u32::MAX,
    }));
    let Output::Device(device::Output::Insert { device: fx }) = out else {
        panic!("{out:?}")
    };

    // A set_param by name, undone: the project keeps the default again.
    set(&mut s, &fx, "ceiling", -9.0);
    assert_eq!(param(&s, &fx, "ceiling"), Some(-9.0));
    s.run(Command::History(history::Command::Undo {}));
    assert_eq!(param(&s, &fx, "ceiling"), Some(-0.3));

    // 1. Dry: bypassed.
    bypass(&mut s, &fx, true);
    drum_loop(&mut s, &track);
    // 2. Pushed 18 dB against -3 dB (under the output protection's -0.3 dB, which then
    //    has nothing to do).
    bypass(&mut s, &fx, false);
    set(&mut s, &fx, "gain", 18.0);
    set(&mut s, &fx, "ceiling", -3.0);
    drum_loop(&mut s, &track);
    // 3. Against -6 dB with the true-peak detection, a manual release.
    set(&mut s, &fx, "ceiling", -6.0);
    set(&mut s, &fx, "true_peak", 1.0);
    set(&mut s, &fx, "release_mode", 0.0);
    set(&mut s, &fx, "release", 0.05);
    drum_loop(&mut s, &track);

    let r = parity(s.scenario());
    r.assert_clean();
    let section = (SECTION_STEPS * STEP_BLOCKS * 128) as f32 / 48_000.0;
    let (dry, pushed, true_peak) = (
        0.0..section,
        // From the second step on: the bypass crossfade and the 20 ms ramps of the gain and
        // of the ceiling take the first hit.
        section + 0.25..2.0 * section,
        // From the first step on: the true-peak switch fades out and in at its start.
        2.0 * section + 0.25..3.0 * section,
    );
    let db = |x: f32| 20.0 * x.log10();
    let (dry_peak, pushed_peak, tp_peak) = (
        r.peak_in(dry.clone()),
        r.peak_in(pushed.clone()),
        r.peak_in(true_peak.clone()),
    );
    let (dry_rms, pushed_rms) = (r.rms_in(dry.clone()), r.rms_in(pushed.clone()));
    println!(
        "dry: peak {:.2} dB rms {:.2} dB; pushed 18 dB, ceiling -3 dB: peak {:.2} dB rms {:.2} dB; ceiling -6 dB with true peak: peak {:.2} dB",
        db(dry_peak),
        db(dry_rms),
        db(pushed_peak),
        db(pushed_rms),
        db(tp_peak),
    );
    r.assert_audible(dry.clone(), 0.05);
    // The ceiling holds, the loop sits on it and is much louder than dry.
    assert!(db(pushed_peak) <= -3.0 + 1e-4, "{}", db(pushed_peak));
    assert!(db(pushed_peak) > -3.5);
    assert!(db(pushed_rms) - db(dry_rms) > 6.0);
    assert!(db(tp_peak) <= -6.0 + 1e-4, "{}", db(tp_peak));
    assert!(db(tp_peak) > -6.5);
    r.write_wav("ceiling");
}
