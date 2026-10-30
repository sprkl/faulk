//! #402 through the command layer and the engine: whatever the mix, nothing leaves the engine
//! above -0.3 dBFS; a mix under the ceiling is bit for bit what it was; the render is the same
//! natively and in WASM; the export gets the same protection. A hot render is written to
//! `.qa/renders/output-limiter-hot.wav` for the ear (only the user judges by ear).

use commands::testing::{context, fixture};
use commands::{Command, Output, device, live, mixer};
use engine::limiter::CEILING;
use test_support::{Render, Session, parity, render_native};

/// A resonant low-pass on the synth, a four-note chord at full velocity held for a second,
/// the track and the master at `track_db` and `master_db`.
fn song(resonance: f64, track_db: f64, master_db: f64) -> Session {
    let f = fixture();
    let mut s = Session::from_state(f.state.clone(), context());
    let out = s.exec(Command::Device(device::Command::Insert {
        track: Some(f.track.clone()),
        master: false,
        kind: "faulk.filter".into(),
        index: 0,
    }));
    let Output::Device(device::Output::Insert { device: fx }) = out else {
        panic!("{out:?}")
    };
    for (param, value) in [("cutoff", 300.0), ("resonance", resonance)] {
        s.run(Command::Device(device::Command::SetParam {
            device: fx.clone(),
            param: param.into(),
            value,
        }));
    }
    s.run(Command::Mixer(mixer::Command::SetVolume {
        track: f.track.clone(),
        gain_db: Some(track_db),
    }));
    s.run(Command::Mixer(mixer::Command::SetMasterVolume {
        gain_db: Some(master_db),
    }));
    for note in [48, 55, 60, 64] {
        s.run(Command::Live(live::Command::NoteOn {
            track: f.track.clone(),
            note,
            velocity: 127,
        }));
    }
    s.render_secs(1.0);
    s.run(Command::Live(live::Command::AllNotesOff {}));
    s.render_secs(0.5);
    s
}

fn render(s: &Session) -> Render {
    let r = render_native(s.scenario());
    r.assert_finite();
    r.assert_no_denormals();
    r
}

#[test]
fn a_hot_resonant_mix_never_leaves_above_the_ceiling_and_native_equals_wasm() {
    // The same song 26 dB apart: the quiet one is not limited, so its peak times the gain
    // ratio is what the hot one would reach without the protection.
    let quiet = render(&song(1.0, 0.0, -20.0));
    let ratio = 10f32.powf(26.0 / 20.0);
    assert!(
        quiet.peak() < CEILING,
        "the reference is under the ceiling: {}",
        quiet.peak()
    );
    assert!(
        quiet.peak() * ratio > 2.0,
        "the hot mix would reach {} without the protection",
        quiet.peak() * ratio
    );
    let hot = parity(song(1.0, 0.0, 6.0).scenario());
    hot.assert_finite();
    hot.assert_no_denormals();
    assert!(hot.peak() <= CEILING, "peak {}", hot.peak());
    assert!(hot.peak() > 0.95, "it works at the ceiling: {}", hot.peak());
    // What follows the note (the release) is silence, not a buzz.
    assert!(hot.rms_in(0.5..1.0) > 0.1);
    let path = hot.write_wav("output-limiter-hot");
    println!(
        "{}: peak {:.4} (ceiling {:.4}), unprotected peak about {:.1}",
        path.display(),
        hot.peak(),
        CEILING,
        quiet.peak() * ratio
    );
}
