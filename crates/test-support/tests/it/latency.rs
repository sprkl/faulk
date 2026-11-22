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

// ---------------------------------------------------------------- #765

/// A project made through the commands (#765): the fixture's synth track, a Ceiling below its
/// ceiling with a look-ahead of `lookahead` seconds after the synth, and a Filter whose cutoff
/// a lane sweeps with hard steps and ramps, after the Ceiling on the track or (`on_master`) on
/// the master; played from the start, a held note, 1.5 s. The render, natively and in WASM
/// (`parity`).
fn filter_behind_ceiling(lookahead: f64, on_master: bool) -> test_support::Render {
    use commands::{automation, transport};
    use project::{AutomationTarget, Shape, Tick};
    let f = fixture();
    let mut s = Session::from_state(f.state.clone(), context());
    let fx = ceiling(&mut s, Some(&f.track));
    set(&mut s, &fx, "ceiling", 0.0);
    set(&mut s, &fx, "lookahead", lookahead);
    let out = s.exec(Command::Device(device::Command::Insert {
        track: (!on_master).then(|| f.track.clone()),
        master: on_master,
        kind: "faulk.filter".into(),
        index: u32::MAX,
    }));
    let Output::Device(device::Output::Insert { device: filter }) = out else {
        panic!("{out:?}")
    };
    set(&mut s, &filter, "mode", 0.0);
    set(&mut s, &filter, "resonance", 0.7);
    let Output::Automation(automation::Output::LaneCreate { lane }) =
        s.exec(Command::Automation(automation::Command::LaneCreate {
            track: (!on_master).then(|| f.track.clone()),
            master: on_master,
            target: AutomationTarget::Device {
                device: filter.clone(),
                param: "cutoff".into(),
            },
        }))
    else {
        panic!("a lane")
    };
    // 1 920 ticks a second at 120 BPM: hard steps on attacks, then a ramp.
    let point = |tick: i64, value: f64, shape: Shape| automation::Point {
        tick: Tick(tick),
        value,
        shape,
    };
    s.run(Command::Automation(automation::Command::PointsAdd {
        lane,
        points: vec![
            point(0, 300.0, Shape::Step),
            point(480, 6_000.0, Shape::Step),
            point(960, 250.0, Shape::Step),
            point(1_200, 400.0, Shape::Linear),
            point(2_400, 9_000.0, Shape::Step),
        ],
    }));
    s.render_blocks(8);
    s.run(Command::Transport(transport::Command::Play {}));
    s.run(Command::Live(live::Command::NoteOn {
        track: f.track.clone(),
        note: 45,
        velocity: 100,
    }));
    s.render_secs(1.5);
    let r = parity(s.scenario());
    r.assert_clean();
    r
}

#[test]
fn a_filter_swept_behind_a_ceiling_renders_without_its_latency_one_latency_later() {
    // The Filter after the Ceiling on the track, and on the master behind the track's Ceiling
    // (the sum reaches the master that late): its sweep lands on the sound it belongs to.
    let n = 72;
    for on_master in [false, true] {
        let plain = filter_behind_ceiling(0.0, on_master);
        let latent = filter_behind_ceiling(0.0015, on_master);
        assert!(plain.peak() > 0.05, "the note sounds: {}", plain.peak());
        let len = plain.frames() - n;
        let off =
            |a: &[f32], b: &[f32]| a.iter().zip(b).fold(0f32, |m, (x, y)| m.max((x - y).abs()));
        let (off_l, off_r) = (
            off(&latent.left[n..], &plain.left[..len]),
            off(&latent.right[n..], &plain.right[..len]),
        );
        let on = if on_master { "master" } else { "track" };
        println!(
            "Filter on the {on} behind a Ceiling of {n} frames, shifted back: left off by \
             {off_l:e}, right by {off_r:e} (native; WASM within the parity tolerance)"
        );
        assert_eq!(off_l, 0.0, "{on}");
        assert_eq!(off_r, 0.0, "{on}");
        // The control: unshifted, the sweep's steps make the two renders differ.
        let early = off(&latent.left, &plain.left);
        assert!(early > 0.01, "{on}: the latency moves the render: {early}");
    }
}

// ---------------------------------------------------------------- #761

#[test]
fn a_ceiling_inserted_and_removed_on_the_shorter_track_leaves_it_aligned_all_along() {
    // Two drum tracks, the right one through a Ceiling of 5 ms (the longest chain). The left
    // one gets a Ceiling of 1.5 ms at the end of its chain during the hits, bypassed, removed,
    // and inserted and removed again: its timing never changes (the slot and the track's delay
    // switch at the same frame), so the two channels stay equal to the sample all along,
    // natively and in WASM.
    let mut s = Session::from_state(fixture().state, context());
    let left = drums(&mut s, -1.0);
    let right = drums(&mut s, 1.0);
    let longest = ceiling(&mut s, Some(&right));
    set(&mut s, &longest, "ceiling", 0.0);
    set(&mut s, &longest, "lookahead", 0.005);
    s.render_blocks(20);
    let insert = |s: &mut Session| {
        let fx = ceiling(s, Some(&left));
        set(s, &fx, "ceiling", 0.0);
        fx
    };
    let remove = |s: &mut Session, fx: DeviceId| {
        s.run(Command::Device(device::Command::Remove { device: fx }));
    };
    hits(&mut s, &[&left, &right]);
    let fx = insert(&mut s);
    hits(&mut s, &[&left, &right]);
    s.run(Command::Device(device::Command::SetBypass {
        device: fx.clone(),
        bypass: true,
    }));
    hits(&mut s, &[&left, &right]);
    remove(&mut s, fx);
    hits(&mut s, &[&left, &right]);
    let fx = insert(&mut s);
    hits(&mut s, &[&left, &right]);
    remove(&mut s, fx);
    hits(&mut s, &[&left, &right]);

    let r = parity(s.scenario());
    r.assert_clean();
    assert!(r.peak() > 0.1, "the drums sound: {}", r.peak());
    let off = mismatch(&r, 0.0..r.frames() as f32 / r.sample_rate);
    println!("a Ceiling inserted and removed on the shorter track: left − right at most {off:e}");
    assert_eq!(off, 0.0, "aligned all along");
}
