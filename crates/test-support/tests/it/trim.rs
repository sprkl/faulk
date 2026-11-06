//! Trim (#693) through the command layer and the engine: `device.insert` puts `faulk.trim` at
//! the head of a track's chain, before the synth; `device.set_param` sets each of its
//! parameters by name (schema, description and undo checked by `commands::testing::check`);
//! a short live note comes out an eighth note long at 120 BPM, natively and in WASM, and
//! bypassed Trim leaves the note as played. The render is written to
//! `.qa/renders/trim-eighth.wav` for the ear. (The device itself is tested in
//! `crates/engine/tests/it/trim.rs`.)

use commands::testing::{check, context, fixture};
use commands::{Command, Output, device, live, transport};
use engine::devices::trim::DESCRIPTOR;
use project::{DeviceId, DeviceRole};
use test_support::{Session, parity};

fn insert(s: &mut Session, track: &project::TrackId) -> DeviceId {
    let out = s.exec(Command::Device(device::Command::Insert {
        track: Some(track.clone()),
        master: false,
        kind: "faulk.trim".into(),
        index: 0,
    }));
    let Output::Device(device::Output::Insert { device: fx }) = out else {
        panic!("{out:?}")
    };
    fx
}

fn set(device: &DeviceId, param: &str, value: f64) -> Command {
    Command::Device(device::Command::SetParam {
        device: device.clone(),
        param: param.into(),
        value,
    })
}

#[test]
fn trim_goes_before_the_synth_and_each_parameter_is_set_by_name_and_undone() {
    let f = fixture();
    let insert_cmd = Command::Device(device::Command::Insert {
        track: Some(f.track.clone()),
        master: false,
        kind: "faulk.trim".into(),
        index: 0,
    });
    check(&insert_cmd, &f.state, &context()).unwrap();
    let mut s = Session::from_state(f.state.clone(), context());
    let fx = insert(&mut s, &f.track);
    let chain = s.state().project.tracks[&f.track].devices_in_order();
    assert_eq!(chain[0].0, &fx);
    assert_eq!(chain[0].1.role, DeviceRole::MidiEffect);
    assert_eq!(chain[1].0, &f.device, "the synth comes after Trim");
    for p in DESCRIPTOR.params {
        for value in [p.min, p.max, p.min + (p.max - p.min) * 0.37] {
            let cmd = set(&fx, p.key, value);
            let current = s.state().project.tracks[&f.track].devices[&fx]
                .params
                .get(p.key)
                .copied()
                .unwrap_or(p.default);
            // An edit must change something: the check runs on a new value.
            if p.sanitize_f64(value) == Some(current) {
                continue;
            }
            check(&cmd, s.state(), s.ctx()).unwrap_or_else(|e| panic!("{} = {value}: {e}", p.key));
            let out = s.exec(cmd);
            let Output::Device(device::Output::SetParam { value: stored, .. }) = out else {
                panic!("{out:?}")
            };
            assert_eq!(Some(stored), p.sanitize_f64(value), "{} = {value}", p.key);
            assert_eq!(
                s.state().project.tracks[&f.track].devices[&fx].params[p.key],
                stored
            );
        }
    }
    // An unknown parameter is refused.
    assert!(commands::testing::check_refusal(&set(&fx, "length", 1.0), s.state(), s.ctx()).is_ok());
}

/// A live note of 20 ms at 120 BPM, through Trim at its defaults but an eighth note, or
/// bypassed; the synth's release is cut to 10 ms.
fn render(bypass: bool) -> test_support::Render {
    let f = fixture();
    let mut s = Session::from_state(f.state.clone(), context());
    let fx = insert(&mut s, &f.track);
    s.run(set(&fx, "division", 3.0));
    s.run(set(&f.device, "release", 0.01));
    if bypass {
        s.run(Command::Device(device::Command::SetBypass {
            device: fx.clone(),
            bypass: true,
        }));
    }
    s.run(Command::Transport(transport::Command::Play {}));
    s.render_secs(0.1);
    s.run(Command::Live(live::Command::NoteOn {
        track: f.track.clone(),
        note: 57,
        velocity: 110,
    }));
    s.render_secs(0.02);
    s.run(Command::Live(live::Command::NoteOff {
        track: f.track.clone(),
        note: 57,
    }));
    s.render_secs(0.6);
    parity(s.scenario())
}

#[test]
fn a_short_note_comes_out_an_eighth_long_at_120_bpm_natively_and_in_wasm() {
    let r = render(false);
    r.assert_clean();
    // The note starts at 0.1 s and lasts 250 ms, then a 10 ms release.
    r.assert_audible(0.12..0.34, 0.01);
    r.assert_silent(0.37..0.72);
    r.write_wav("trim-eighth");

    let dry = render(true);
    dry.assert_clean();
    dry.assert_audible(0.1..0.115, 0.01);
    dry.assert_silent(0.15..0.72);
}
