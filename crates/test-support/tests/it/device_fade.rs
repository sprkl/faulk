//! #233 through the command layer and the engine: while a low sine plays, an effect is
//! inserted, bypassed, put back, rebuilt by an insert in front of it, removed, and the
//! instrument is replaced. Each change fades: the largest step between two neighbouring
//! samples around it stays within what a 5 ms linear crossfade between the steady signal
//! before and the one after allows. The render is the same natively and in WASM, and is
//! written to `.qa/renders/device-fade.wav` for the ear.

use commands::testing::{context, fixture};
use commands::{Command, Output, device, live, track};
use engine::graph::fade_frames;
use test_support::{QUANTUM, SAMPLE_RATE, Session, parity};

/// Blocks between two changes (0.5 s).
const BLOCKS: u32 = 188;

/// Largest difference between two neighbouring samples.
fn max_step(x: &[f32]) -> f32 {
    x.windows(2).fold(0f32, |m, w| m.max((w[1] - w[0]).abs()))
}

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0f32, |m, s| m.max(s.abs()))
}

#[test]
fn every_device_change_during_a_note_fades() {
    let f = fixture();
    let mut s = Session::from_state(f.state.clone(), context());
    let synth = f.device.clone();
    // A sustained sine at 110 Hz: from one sample to the next it moves by at most 1.5 % of
    // its level, so a cut stands out.
    s.run(Command::Device(device::Command::SetParam {
        device: synth.clone(),
        param: "waveform".into(),
        value: 0.0,
    }));
    s.run(Command::Live(live::Command::NoteOn {
        track: f.track.clone(),
        note: 45,
        velocity: 110,
    }));
    s.render_blocks(BLOCKS);
    let mut changes: Vec<(&str, usize)> = Vec::new();
    let mut at = BLOCKS as usize * QUANTUM as usize;
    let insert = |s: &mut Session, kind: &str| {
        let out = s.exec(Command::Device(device::Command::Insert {
            track: Some(f.track.clone()),
            master: false,
            kind: kind.into(),
            index: 0,
        }));
        let Output::Device(device::Output::Insert { device }) = out else {
            panic!("{out:?}")
        };
        device
    };
    let echo = insert(&mut s, "faulk.echo");
    changes.push(("insert the echo", at));
    // Two changes a second apart: the echo's repeats (300 ms, feedback 0.4) settle into a
    // steady sine before the next change.
    s.render_blocks(2 * BLOCKS);
    at += 2 * BLOCKS as usize * QUANTUM as usize;
    let bypass = |s: &mut Session, device: &project::DeviceId, bypass: bool| {
        s.run(Command::Device(device::Command::SetBypass {
            device: device.clone(),
            bypass,
        }));
    };
    bypass(&mut s, &echo, true);
    changes.push(("bypass the echo", at));
    s.render_blocks(BLOCKS);
    at += BLOCKS as usize * QUANTUM as usize;
    bypass(&mut s, &echo, false);
    changes.push(("put the echo back", at));
    s.render_blocks(2 * BLOCKS);
    at += 2 * BLOCKS as usize * QUANTUM as usize;
    // A filter in front: the echo is built again one slot further (a crossfade from the old
    // echo to the new one).
    insert(&mut s, "faulk.filter");
    changes.push(("insert a filter in front of the echo", at));
    s.render_blocks(2 * BLOCKS);
    at += 2 * BLOCKS as usize * QUANTUM as usize;
    s.run(Command::Device(device::Command::Remove {
        device: echo.clone(),
    }));
    changes.push(("remove the echo", at));
    s.render_blocks(BLOCKS);
    at += BLOCKS as usize * QUANTUM as usize;
    bypass(&mut s, &synth, true);
    changes.push(("bypass the synth", at));
    s.render_blocks(BLOCKS);
    at += BLOCKS as usize * QUANTUM as usize;
    bypass(&mut s, &synth, false);
    s.run(Command::Live(live::Command::NoteOn {
        track: f.track.clone(),
        note: 45,
        velocity: 110,
    }));
    s.render_blocks(BLOCKS);
    at += BLOCKS as usize * QUANTUM as usize;
    let out = s.exec(Command::Track(track::Command::SetInstrument {
        track: f.track.clone(),
        instrument: "faulk.drums".into(),
    }));
    assert!(matches!(out, Output::Track(_)), "{out:?}");
    changes.push(("replace the synth by the drums", at));
    s.render_blocks(BLOCKS);

    let r = parity(s.scenario());
    r.assert_clean();
    r.write_wav("device-fade");
    let n = fade_frames(SAMPLE_RATE) as usize;
    let tenth = (SAMPLE_RATE / 10.0) as usize;
    for (name, at) in changes {
        for channel in [&r.left, &r.right] {
            // The steady signals on both sides, past the fade, and the crossfade's own slope:
            // `(after − before) / n` at most, which `peak before + peak after` bounds.
            let before = &channel[at - tenth..at];
            let after = &channel[at + 4 * n..at + 4 * n + tenth];
            let bound = max_step(before).max(max_step(after)) * 1.05
                + 1.5 * (peak(before) + peak(after)) / n as f32
                + 1e-6;
            let around = &channel[at - QUANTUM as usize..at + 4 * n];
            let step = max_step(around);
            assert!(
                step <= bound,
                "{name}: a step of {step} at frame {at}, bound {bound}"
            );
        }
    }
}
