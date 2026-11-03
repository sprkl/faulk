//! #412 through the command layer and the engine: `mixer.set_master_pan` is audible (the
//! channels move apart), centre leaves both channels equal, undo brings the balance back,
//! and the render is the same natively and in WASM.

use commands::testing::{context, fixture};
use commands::{Command, history, live, mixer};
use test_support::{Render, Session, parity, render_native};

fn song(pan: Option<f64>, undo: bool) -> Session {
    let f = fixture();
    let mut s = Session::from_state(f.state.clone(), context());
    if let Some(pan) = pan {
        s.run(Command::Mixer(mixer::Command::SetMasterPan { pan }));
        if undo {
            s.run(Command::History(history::Command::Undo {}));
        }
    }
    s.run(Command::Live(live::Command::NoteOn {
        track: f.track.clone(),
        note: 60,
        velocity: 120,
    }));
    s.render_secs(1.0);
    s.run(Command::Live(live::Command::AllNotesOff {}));
    s.render_secs(0.3);
    s
}

fn rms(channel: &[f32], r: &Render) -> f32 {
    let from = (0.3 * r.sample_rate) as usize;
    let to = (0.9 * r.sample_rate) as usize;
    let part = &channel[from..to];
    (part.iter().map(|v| v * v).sum::<f32>() / part.len() as f32).sqrt()
}

#[test]
fn a_master_pan_moves_the_balance_and_native_equals_wasm() {
    let centre = render_native(song(None, false).scenario());
    centre.assert_clean();
    let (cl, cr) = (rms(&centre.left, &centre), rms(&centre.right, &centre));
    assert!(cl > 0.001, "the song is silent");
    assert!(
        (cl - cr).abs() < 0.01 * cl,
        "centre is not balanced: {cl} {cr}"
    );

    let left = parity(song(Some(-1.0), false).scenario());
    left.assert_clean();
    let (ll, lr) = (rms(&left.left, &left), rms(&left.right, &left));
    assert!(lr < 0.05 * ll, "hard left leaks to the right: {ll} {lr}");

    let right = render_native(song(Some(1.0), false).scenario());
    let (rl, rr) = (rms(&right.left, &right), rms(&right.right, &right));
    assert!(rl < 0.05 * rr, "hard right leaks to the left: {rl} {rr}");

    // Undo puts the balance back.
    let back = render_native(song(Some(-1.0), true).scenario());
    let (bl, br) = (rms(&back.left, &back), rms(&back.right, &back));
    assert!(
        (bl - br).abs() < 0.01 * bl,
        "undo did not recentre: {bl} {br}"
    );
}
