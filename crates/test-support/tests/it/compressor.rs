//! DV-5 through the command layer and the engine: `device.insert` puts the compressor after
//! the drums of a track, `device.set_param` (by name) and `device.set_bypass` move it, and
//! the render is the same natively and in WASM. The render, the drum loop dry, with a gentle
//! compression and with a heavy one, is written to `.qa/renders/compressor.wav` for the ear.
//! (The device itself is tested in `crates/engine/tests/it/compressor.rs`.)

use commands::testing::{context, fixture};
use commands::{Command, Output, device, live, track};
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
    // (kick, snare, hat velocity) per eighth note of a bar, played twice.
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

fn drums_track(s: &mut Session) -> TrackId {
    let out = s.exec(Command::Track(track::Command::Create {
        kind: TrackKind::Midi,
        instrument: Some("faulk.drums".into()),
        name: None,
        color: None,
    }));
    let Output::Track(track::Output::Create { track, .. }) = out else {
        panic!("{out:?}")
    };
    track
}

#[test]
fn the_compressor_tames_a_drum_loop_dry_gentle_then_heavy() {
    let mut s = Session::from_state(fixture().state, context());
    let track = drums_track(&mut s);
    let out = s.exec(Command::Device(device::Command::Insert {
        track: Some(track.clone()),
        master: false,
        kind: "faulk.compressor".into(),
        index: 0,
    }));
    let Output::Device(device::Output::Insert { device: fx }) = out else {
        panic!("{out:?}")
    };

    // 1. Dry: the compressor is bypassed.
    bypass(&mut s, &fx, true);
    drum_loop(&mut s, &track);
    // 2. Gentle: 2:1 from -14 dB, slow, a little makeup.
    bypass(&mut s, &fx, false);
    for (param, value) in [
        ("threshold", -14.0),
        ("ratio", 2.0),
        ("attack", 0.02),
        ("release", 0.2),
        ("makeup", 2.0),
    ] {
        set(&mut s, &fx, param, value);
    }
    drum_loop(&mut s, &track);
    // 3. Heavy: 8:1 from -26 dB, 5 ms attack, 6 dB of makeup (a bigger makeup lets the
    // first milliseconds of each hit, which the attack has not yet turned down, clip).
    for (param, value) in [
        ("threshold", -26.0),
        ("ratio", 8.0),
        ("attack", 0.005),
        ("release", 0.1),
        ("makeup", 6.0),
    ] {
        set(&mut s, &fx, param, value);
    }
    drum_loop(&mut s, &track);

    let r = parity(s.scenario());
    r.assert_clean();
    let section = (SECTION_STEPS * STEP_BLOCKS * 128) as f32 / 48_000.0;
    let (dry, gentle, heavy) = (
        0.0..section,
        section..2.0 * section,
        2.0 * section..3.0 * section,
    );
    // How unevenly loud the steps of the loop are: the rms of the loudest step over the rms
    // of the quietest one (a hat alone), in decibels.
    let step = (STEP_BLOCKS * 128) as f32 / 48_000.0;
    let spread = |w: &std::ops::Range<f32>| {
        let levels: Vec<f32> = (0..SECTION_STEPS)
            .map(|k| {
                let from = w.start + k as f32 * step;
                r.rms_in(from..from + step)
            })
            .collect();
        let max = levels.iter().cloned().fold(0f32, f32::max);
        let min = levels.iter().cloned().fold(f32::MAX, f32::min);
        20.0 * (max / min).log10()
    };
    let (s_dry, s_gentle, s_heavy) = (spread(&dry), spread(&gentle), spread(&heavy));
    println!(
        "dry: peak {:.3} rms {:.4} step spread {s_dry:.1} dB
gentle: peak {:.3} rms {:.4} step spread {s_gentle:.1} dB
heavy: peak {:.3} rms {:.4} step spread {s_heavy:.1} dB",
        r.peak_in(dry.clone()),
        r.rms_in(dry.clone()),
        r.peak_in(gentle.clone()),
        r.rms_in(gentle.clone()),
        r.peak_in(heavy.clone()),
        r.rms_in(heavy.clone()),
    );
    r.assert_audible(dry.clone(), 0.05);
    // The more it compresses, the more even the steps are.
    assert!(
        s_dry > s_gentle && s_gentle > s_heavy,
        "{s_dry} {s_gentle} {s_heavy}"
    );

    // Bypassed, the loop is the drums alone: the same section of a session without any
    // compressor is identical.
    let mut plain = Session::from_state(fixture().state, context());
    let t2 = drums_track(&mut plain);
    drum_loop(&mut plain, &t2);
    let reference = parity(plain.scenario());
    let frames = (section * 48_000.0) as usize;
    let drift = (0..frames)
        .map(|i| {
            (r.left[i] - reference.left[i])
                .abs()
                .max((r.right[i] - reference.right[i]).abs())
        })
        .fold(0f32, f32::max);
    assert!(
        drift < 1e-6,
        "bypassed, the compressor changes nothing: {drift}"
    );
    r.write_wav("compressor");
}
