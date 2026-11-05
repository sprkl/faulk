//! #749 through the command layer and the engine: two drum tracks playing the same hits, one
//! hard left and one hard right, the right one through a Ceiling below its ceiling
//! (`device.insert`), bypassed and back (`device.set_bypass`), its look-ahead moved
//! (`device.set_param`), a Ceiling on the master with the true-peak detection. The render is
//! the same natively and in WASM, and its two channels are equal to the sample outside the
//! look-ahead's own switch (the engine compensates the Ceiling's latency). The engine's own
//! renders are in `crates/engine/tests/it/latency.rs`.

use commands::testing::{context, fixture};
use commands::{Command, Output, device, live, mixer, track};
use project::{DeviceId, TrackId, TrackKind};
use test_support::{Session, parity};

/// One step, in blocks of 128 frames: about 0.25 s at 48 kHz.
const STEP_BLOCKS: u32 = 94;

fn drums(s: &mut Session, pan: f64) -> TrackId {
    let out = s.exec(Command::Track(track::Command::Create {
        kind: TrackKind::Midi,
        instrument: Some("faulk.drums".into()),
        name: None,
        color: None,
    }));
    let Output::Track(track::Output::Create { track, .. }) = out else {
        panic!("{out:?}")
    };
    s.run(Command::Mixer(mixer::Command::SetPan {
        track: track.clone(),
        pan,
    }));
    track
}

fn ceiling(s: &mut Session, track: Option<&TrackId>) -> DeviceId {
    let out = s.exec(Command::Device(device::Command::Insert {
        track: track.cloned(),
        master: track.is_none(),
        kind: "faulk.ceiling".into(),
        index: u32::MAX,
    }));
    let Output::Device(device::Output::Insert { device }) = out else {
        panic!("{out:?}")
    };
    device
}

fn set(s: &mut Session, fx: &DeviceId, param: &str, value: f64) {
    s.run(Command::Device(device::Command::SetParam {
        device: fx.clone(),
        param: param.into(),
        value,
    }));
}

/// Eight steps of kick, snare and hat on both tracks at once.
fn hits(s: &mut Session, tracks: &[&TrackId]) {
    for step in 0..8u8 {
        for t in tracks {
            for (note, velocity) in [(36, 110), (38, 100), (42, 70)] {
                if note == 42 || (note == 36) == (step % 2 == 0) {
                    s.run(Command::Live(live::Command::NoteOn {
                        track: (*t).clone(),
                        note,
                        velocity,
                    }));
                }
            }
        }
        s.render_blocks(STEP_BLOCKS);
    }
}

/// The largest difference between the channels over `window` (seconds).
fn mismatch(r: &test_support::Render, window: std::ops::Range<f32>) -> f32 {
    let at = |t: f32| ((t * r.sample_rate) as usize).min(r.frames());
    let (from, to) = (at(window.start), at(window.end));
    r.left[from..to]
        .iter()
        .zip(&r.right[from..to])
        .fold(0f32, |m, (a, b)| m.max((a - b).abs()))
}

#[test]
fn two_tracks_one_through_a_ceiling_stay_aligned_through_the_commands() {
    let mut s = Session::from_state(fixture().state, context());
    let left = drums(&mut s, -1.0);
    let right = drums(&mut s, 1.0);
    let fx = ceiling(&mut s, Some(&right));
    set(&mut s, &fx, "ceiling", 0.0);
    let master = ceiling(&mut s, None);
    set(&mut s, &master, "ceiling", 0.0);
    set(&mut s, &master, "true_peak", 1.0);
    s.render_blocks(20);
    let step = STEP_BLOCKS as f32 * 128.0 / 48_000.0;
    // 1. Aligned; 2. bypassed and back during the hits; 3. a longer look-ahead.
    hits(&mut s, &[&left, &right]);
    s.run(Command::Device(device::Command::SetBypass {
        device: fx.clone(),
        bypass: true,
    }));
    hits(&mut s, &[&left, &right]);
    s.run(Command::Device(device::Command::SetBypass {
        device: fx.clone(),
        bypass: false,
    }));
    hits(&mut s, &[&left, &right]);
    set(&mut s, &fx, "lookahead", 0.005);
    hits(&mut s, &[&left, &right]);

    let r = parity(s.scenario());
    r.assert_clean();
    let start = 20.0 * 128.0 / 48_000.0;
    let section = 8.0 * step;
    assert!(r.peak() > 0.1, "the drums sound: {}", r.peak());
    // Every section but the look-ahead's first step (its settle, fades and silence).
    let aligned = [
        start..start + 3.0 * section,
        start + 3.0 * section + step..start + 4.0 * section,
    ];
    for window in aligned {
        let off = mismatch(&r, window.clone());
        println!("{window:?}: left − right at most {off:e}");
        assert_eq!(off, 0.0, "{window:?}");
    }
}
