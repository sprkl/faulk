//! DV-4 through the command layer and the engine: `device.insert` puts the reverb after the
//! synth of a track, `device.set_param` moves its size and `device.set_bypass` cuts a ringing
//! tail. The render is written to `.qa/renders/reverb.wav` for the ear: the same hit dry,
//! then with a small, a medium and a large size, then a large one cut by the bypass.

use commands::testing::{context, fixture};
use commands::{Command, Output, device, live};
use test_support::{Session, render_native};

/// A short chord (0.08 s) then silence until `rest` seconds after its start.
fn hit(s: &mut Session, track: &project::TrackId, rest: f32) {
    for pitch in [57u8, 64, 69] {
        s.run(Command::Live(live::Command::NoteOn {
            track: track.clone(),
            note: pitch,
            velocity: 110,
        }));
    }
    s.render_secs(0.08);
    s.run(Command::Live(live::Command::AllNotesOff {}));
    s.render_secs(rest - 0.08);
}

#[test]
fn the_reverb_has_a_tail_that_follows_its_size_and_the_bypass() {
    let f = fixture();
    let mut s = Session::from_state(f.state.clone(), context());
    for (param, value) in [("attack", 0.002), ("release", 0.01), ("decay", 0.15)] {
        s.run(Command::Device(device::Command::SetParam {
            device: f.device.clone(),
            param: param.into(),
            value,
        }));
    }
    let out = s.exec(Command::Device(device::Command::Insert {
        track: Some(f.track.clone()),
        master: false,
        kind: "faulk.reverb".into(),
        index: 0,
    }));
    let Output::Device(device::Output::Insert { device: reverb }) = out else {
        panic!("{out:?}")
    };
    let set = |s: &mut Session, param: &str, value: f64| {
        s.run(Command::Device(device::Command::SetParam {
            device: reverb.clone(),
            param: param.into(),
            value,
        }));
    };
    let bypass = |s: &mut Session, on: bool| {
        s.run(Command::Device(device::Command::SetBypass {
            device: reverb.clone(),
            bypass: on,
        }));
    };
    set(&mut s, "mix", 0.35);
    set(&mut s, "damping", 0.4);

    // 1. Dry (bypassed), starts at 0 s.
    bypass(&mut s, true);
    hit(&mut s, &f.track, 4.0);
    // 2. Small, starts at 4.5 s.
    bypass(&mut s, false);
    set(&mut s, "size", 0.15);
    s.render_secs(0.5);
    hit(&mut s, &f.track, 4.0);
    // 3. Medium, starts at 9 s.
    set(&mut s, "size", 0.5);
    s.render_secs(0.5);
    hit(&mut s, &f.track, 4.0);
    // 4. Large, starts at 13.5 s.
    set(&mut s, "size", 0.95);
    s.render_secs(0.5);
    hit(&mut s, &f.track, 8.0);

    // 5. A large hit at 22 s, bypassed at 22.5 s while its tail rings: the tail is cut.
    set(&mut s, "size", 0.95);
    s.render_secs(0.5);
    hit(&mut s, &f.track, 0.5);
    bypass(&mut s, true);
    s.render_secs(1.0);

    // Native only: the interpreter takes minutes on 30 s; the parity is `reverb_parity.rs`.
    let r = render_native(s.scenario());
    r.assert_clean();
    // The dry hit leaves nothing after its release.
    r.assert_audible(0.0..0.08, 0.05);
    r.assert_silent(0.3..4.0);
    // Each reverb tail rings after the note, longer with the size.
    let small = r.rms_in(4.8..5.1);
    let medium = r.rms_in(9.3..9.6);
    let large = r.rms_in(13.8..14.1);
    println!("rms 0.3 s after the hit: small {small}, medium {medium}, large {large}");
    assert!(small > 1e-4 && medium > 1e-3 && large > 1e-3);
    // Later on, the small room is gone and the large one still rings.
    let late = |from: f32| r.rms_in(from + 2.0..from + 2.3);
    println!(
        "rms 2 s after the hit: small {}, large {}",
        late(4.5),
        late(13.5)
    );
    assert!(late(4.5) < 0.1 * late(13.5));
    assert!(late(13.5) > 1e-4);
    r.assert_audible(22.3..22.5, 0.002);
    r.assert_silent(22.6..23.4);
    r.write_wav("reverb");
}
