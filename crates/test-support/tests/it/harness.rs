//! The harness on the reference projects: the fixtures are what the command layer builds,
//! the renders say what the projects are meant to sound like, and a refused command or a
//! stale fixture fails.

use test_support::{Scenario, Session, Step, reference, render_native};

#[test]
fn the_fixtures_are_what_the_command_layer_records() {
    for r in reference::ALL {
        reference::load(r);
    }
}

#[test]
fn the_scenario_format_round_trips_and_refuses_truncation() {
    let s = reference::chord();
    let bytes = s.encode();
    assert_eq!(Scenario::decode(&bytes), Some(s));
    assert_eq!(Scenario::decode(&bytes[..bytes.len() - 1]), None);
    assert_eq!(Scenario::decode(&[7, 0, 0, 0, 0]), None);
    assert_eq!(Scenario::decode(&[]), Some(Scenario::default()));
}

#[test]
fn the_chord_starts_when_played_and_ends_in_silence() {
    let r = render_native(&reference::load(reference::get("chord")));
    r.assert_clean();
    let lead = reference::LEAD_IN;
    r.assert_silent(0.0..lead);
    r.assert_onset_at(lead, 0.005, 0.001, 0.0);
    r.assert_audible(lead + 0.2..lead + 1.0, 0.1);
    // 50 ms release after the notes off at lead + 1.0 s.
    r.assert_silent(lead + 1.2..r.seconds());
    // The chord is A minor: the three notes are in the spectrum, the fourth is not.
    let sp = r.spectrum(lead + 0.3..lead + 1.0);
    for hz in [220.0, 261.63, 329.63] {
        assert!(sp.peak_in(hz - 3.0..hz + 3.0) > 0.03, "{hz} Hz is missing");
    }
    assert!(
        sp.peak_in(430.0..450.0) < 0.003,
        "a sine has no 440 Hz here"
    );
}

#[test]
fn the_melody_plays_its_notes_in_order() {
    let r = render_native(&reference::load(reference::get("melody")));
    r.assert_clean();
    let lead = reference::LEAD_IN;
    r.assert_onset_at(lead, 0.005, 0.001, 0.0);
    for (i, hz) in [220.0, 261.63, 329.63, 440.0].into_iter().enumerate() {
        let from = lead + 0.25 * i as f32 + 0.1;
        let f = r.spectrum(from..from + 0.1).dominant_hz();
        assert!((f - hz).abs() < 6.0, "note {i}: {f} Hz, expected {hz} Hz");
    }
}

#[test]
fn the_wav_file_is_written_where_listening_expects_it() {
    let r = render_native(&reference::chord());
    let path = r.write_wav("test-support-chord");
    assert!(
        path.ends_with(".qa/renders/test-support-chord.wav")
            || path.ends_with(".qa\\renders\\test-support-chord.wav")
    );
    assert_eq!(std::fs::read(&path).unwrap(), r.wav());
}

#[test]
#[should_panic(expected = "refused")]
fn a_refused_command_fails_the_test() {
    use commands::{Command, live};
    let mut s = Session::single_synth();
    // An unknown track is refused by the project.
    let ghost = project::TrackId::parse("00000000000000000000000009").unwrap();
    s.run(Command::Live(live::Command::NoteOn {
        track: ghost,
        note: 60,
        velocity: 100,
    }));
}

#[test]
fn render_secs_rounds_up_to_whole_blocks() {
    let mut s = Session::single_synth();
    s.render_secs(0.001);
    let blocks = s.scenario().steps.iter().find_map(|st| match st {
        Step::Render(n) => Some(*n),
        _ => None,
    });
    assert_eq!(blocks, Some(1));
    assert!((s.scenario().seconds() - 128.0 / 48_000.0).abs() < 1e-9);
}

#[test]
fn a_truncated_push_does_not_decode() {
    // A push of 9 bytes that holds 3.
    let mut bytes = vec![0, 9, 0, 0, 0];
    bytes.extend_from_slice(&[1, 2, 3]);
    assert_eq!(Scenario::decode(&bytes), None);
}

#[test]
#[should_panic(expected = "the stream is accepted")]
fn the_native_render_refuses_a_stream_the_engine_refuses() {
    let scenario = Scenario {
        steps: vec![Step::Push(vec![0xFF; 7]), Step::Render(1)],
    };
    render_native(&scenario);
}

#[test]
#[should_panic(expected = "refused a push")]
fn the_wasm_render_refuses_a_stream_the_module_refuses() {
    let scenario = Scenario {
        steps: vec![Step::Push(vec![0xFF; 7]), Step::Render(1)],
    };
    test_support::render_wasm(&scenario);
}
