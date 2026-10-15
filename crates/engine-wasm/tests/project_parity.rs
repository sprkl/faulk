//! The reference projects of the render harness (QA-1), rendered natively and in the WASM
//! module: the two must agree within `PARITY_TOLERANCE`, and the render must carry the
//! sound the project describes.

use test_support::{Session, parity, reference};

#[test]
fn every_reference_project_renders_alike_natively_and_in_wasm() {
    for reference in reference::ALL {
        let render = parity(&reference::load(reference));
        render.assert_clean();
        assert!(render.rms() > 0.01, "{} is silent", reference.name);
    }
}

#[test]
fn a_command_built_note_starts_at_the_same_time_in_both_targets() {
    let mut s = Session::single_synth();
    s.render_secs(0.2).note_on(60, 100).render_secs(0.3);
    let render = parity(s.scenario());
    render.assert_silent(0.0..0.2);
    render.assert_onset_at(0.2, 0.005, 0.001, 0.0);
}
