//! A project built through commands, rendered offline by the harness (QA-1): the engine
//! core's use of `test-support`. The native/WASM comparison of the same references is
//! `crates/engine-wasm/tests/project_parity.rs`.

use test_support::{Session, reference, render_native};

#[test]
fn a_project_built_by_commands_renders_the_notes_it_plays() {
    let mut s = Session::single_synth();
    s.set_param("waveform", 0.0)
        .set_param("release", 0.05)
        .render_secs(0.1)
        .note_on(69, 100)
        .render_secs(0.6)
        .note_off(69)
        .render_secs(0.6);
    let r = render_native(s.scenario());
    r.assert_clean();
    r.assert_silent(0.0..0.1);
    r.assert_onset_at(0.1, 0.005, 0.001, 0.0);
    let f = r.spectrum(0.3..0.7).dominant_hz();
    assert!((f - 440.0).abs() < 3.0, "A4 measured {f} Hz");
    r.assert_silent(1.0..r.seconds());
}

#[test]
fn the_reference_projects_render_clean_and_are_not_silent() {
    for reference in reference::ALL {
        let r = render_native(&reference::load(reference));
        r.assert_clean();
        assert!(r.rms() > 0.01, "{} is silent", reference.name);
    }
}

#[test]
fn a_render_can_be_written_for_listening() {
    let r = render_native(&reference::load(reference::get("melody")));
    let path = r.write_wav("engine-melody");
    assert!(std::fs::metadata(path).unwrap().len() > 44);
}
