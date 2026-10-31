//! The native render, and the native/WASM comparison.

use crate::scenario::{Scenario, Step};
use crate::signal::Render;
use crate::wasm::render_wasm;

pub const SAMPLE_RATE: f32 = 48_000.0;
/// Frames per block: the AudioWorklet's render quantum.
pub const QUANTUM: u32 = 128;
/// libm differs slightly between the native targets and wasm32 (sin, exp2, tanh): the
/// largest sample difference the two builds may show on the same scenario.
pub const PARITY_TOLERANCE: f32 = 1e-5;

/// Renders the scenario through `engine::host`, the native target, flushing the prepare
/// side before each block as the real hosts do. Panics when the engine rejected or
/// skipped a message, or when one is still pending at the end.
pub fn render_native(scenario: &Scenario) -> Render {
    let (mut prepare, mut host) = engine::host::channel(SAMPLE_RATE).expect("the channel opens");
    let mut left = Vec::new();
    let mut right = Vec::new();
    for step in &scenario.steps {
        match step {
            Step::Push(bytes) => prepare.submit(bytes).expect("the stream is accepted"),
            Step::Render(blocks) => {
                for _ in 0..*blocks {
                    // The control thread's regular flush: what waited behind a full ring.
                    prepare.flush();
                    let n = host.render(QUANTUM as usize);
                    left.extend_from_slice(&host.left()[..n]);
                    right.extend_from_slice(&host.right()[..n]);
                }
            }
        }
    }
    prepare.flush();
    assert_eq!(prepare.preparer().rejected(), 0, "messages were rejected");
    assert_eq!(
        prepare.preparer().pending(),
        0,
        "messages are still pending after the last block"
    );
    assert_eq!(host.engine().skipped(), 0, "messages were skipped");
    // The output protection (#402) is the last stage, so `assert_finite`/`assert_clean` on the
    // returned render pass by construction (a NaN is silenced, a runaway clamped). This catches a
    // blow-up on any path the engine ran — the master strip, the master chain, automation, the
    // summing, a device — whose output assertion can no longer see it.
    assert_eq!(
        host.engine().limiter_silenced(),
        0,
        "the output protection silenced non-finite samples: a device or a master-path produced NaN or infinity"
    );
    Render::new(SAMPLE_RATE, left, right)
}

/// Renders the scenario natively and in WASM, asserts the two agree within
/// [`PARITY_TOLERANCE`], and returns the native render.
pub fn parity(scenario: &Scenario) -> Render {
    let native = render_native(scenario);
    let wasm = render_wasm(scenario);
    native.assert_close(&wasm, PARITY_TOLERANCE);
    native
}
