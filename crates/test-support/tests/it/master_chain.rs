//! #364 through the command layer and the engine: the master's chain of effects runs on the
//! sum of the tracks, before the master's fader; a bypassed or empty chain leaves the
//! signal bit for bit; the render is the same natively and in WASM, and the export goes
//! through the chain.

use commands::testing::{context, fixture};
use commands::{Command, Output, device, history, live, mixer};
use project::DeviceId;
use test_support::{Render, Session, parity, render_native};

fn insert_master(s: &mut Session, kind: &str) -> DeviceId {
    let out = s.exec(Command::Device(device::Command::Insert {
        track: None,
        master: true,
        kind: kind.into(),
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

fn bypass(s: &mut Session, fx: &DeviceId, bypass: bool) {
    s.run(Command::Device(device::Command::SetBypass {
        device: fx.clone(),
        bypass,
    }));
}

/// A held synth note for one second, then silence. `chain` prepares the session.
fn song(master_db: Option<f64>, chain: impl Fn(&mut Session)) -> Session {
    let f = fixture();
    let mut s = Session::from_state(f.state.clone(), context());
    if let Some(db) = master_db {
        s.run(Command::Mixer(mixer::Command::SetMasterVolume {
            gain_db: Some(db),
        }));
    }
    chain(&mut s);
    s.run(Command::Live(live::Command::NoteOn {
        track: f.track.clone(),
        note: 60,
        velocity: 120,
    }));
    s.render_secs(1.0);
    s.run(Command::Live(live::Command::AllNotesOff {}));
    s.render_secs(0.5);
    s
}

fn render(s: &Session) -> Render {
    let r = render_native(s.scenario());
    r.assert_clean();
    r
}

#[test]
fn an_empty_or_bypassed_master_chain_leaves_the_signal_bit_for_bit() {
    let dry = render(&song(None, |_| {}));
    assert!(dry.rms() > 0.01);
    let bypassed = render(&song(None, |s| {
        let fx = insert_master(s, "faulk.compressor");
        set(s, &fx, "threshold", -40.0);
        set(s, &fx, "ratio", 20.0);
        bypass(s, &fx, true);
        let echo = insert_master(s, "faulk.echo");
        bypass(s, &echo, true);
    }));
    assert_eq!(dry.left, bypassed.left);
    assert_eq!(dry.right, bypassed.right);
    // Inserted, bypassed, then removed by undo: the same.
    let undone = render(&song(None, |s| {
        let fx = insert_master(s, "faulk.compressor");
        set(s, &fx, "threshold", -40.0);
        s.run(Command::History(history::Command::Undo {}));
        s.run(Command::History(history::Command::Undo {}));
    }));
    assert_eq!(dry.left, undone.left);
}

#[test]
fn a_master_effect_changes_the_mix_before_the_fader_and_native_equals_wasm() {
    let heavy = |s: &mut Session| {
        let fx = insert_master(s, "faulk.compressor");
        set(s, &fx, "threshold", -40.0);
        set(s, &fx, "ratio", 20.0);
        set(s, &fx, "attack", 0.0005);
    };
    // The fader pulls the mix 20 dB down. A chain before the fader sees the full level and
    // squeezes it; a chain after it would see a signal under its threshold and do nothing.
    let plain = render(&song(Some(-20.0), |_| {}));
    let treated = parity(song(Some(-20.0), heavy).scenario());
    treated.assert_clean();
    let ratio = treated.rms_in(0.2..1.0) / plain.rms_in(0.2..1.0);
    assert!(
        ratio < 0.7,
        "the chain did not act before the fader: {ratio}"
    );
    // The fader still acts after the chain: the treated mix is quieter than the unity one.
    let unity = render(&song(None, heavy));
    assert!(treated.rms_in(0.2..1.0) < 0.2 * unity.rms_in(0.2..1.0));
    // No NaN, no denormal anywhere (assert_clean above), and silence stays silence.
    treated.assert_silent(1.45..1.5);
}

#[test]
fn undo_takes_the_master_effect_back_out_of_the_sound() {
    let treated = |undo: bool| {
        render(&song(None, |s| {
            let fx = insert_master(s, "faulk.compressor");
            set(s, &fx, "threshold", -40.0);
            set(s, &fx, "ratio", 20.0);
            if undo {
                s.run(Command::History(history::Command::Undo {}));
                s.run(Command::History(history::Command::Undo {}));
                s.run(Command::History(history::Command::Undo {}));
            }
        }))
    };
    let dry = render(&song(None, |_| {}));
    let with = treated(false);
    let without = treated(true);
    assert!(with.rms_in(0.2..1.0) < 0.8 * dry.rms_in(0.2..1.0));
    assert_eq!(dry.left, without.left);
}
