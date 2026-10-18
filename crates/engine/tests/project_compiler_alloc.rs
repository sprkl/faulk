//! The engine messages the command layer compiles (PR-3) never allocate on the audio path:
//! a project edited through commands (tracks created and deleted, clips, mixer, solo,
//! tempo, metronome, playback, live notes) then resynced by a snapshot, rendered with
//! every block under the allocation detector.

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use commands::Command;
use serde_json::json;
use test_support::{QUANTUM, SAMPLE_RATE, Session, Step};

#[global_allocator]
static ALLOC: AllocDisabler = AllocDisabler;

fn cmd(v: serde_json::Value) -> Command {
    serde_json::from_value(v).unwrap()
}

fn created(out: commands::Output) -> String {
    serde_json::to_value(out).unwrap()["track"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[test]
fn compiled_edits_and_a_resync_render_without_allocating() {
    let mut s = Session::single_synth();
    let synth = s.state().project.tracks.keys().next().unwrap().clone();
    let a = created(s.exec(cmd(json!({"type": "track.create", "kind": "midi"}))));
    let b = created(s.exec(cmd(json!({"type": "track.create", "kind": "midi"}))));
    s.render_secs(0.05);
    s.run(cmd(
        json!({"type": "clip.create", "track": a, "at": {"at": "arrangement", "start": 0}}),
    ));
    s.run(cmd(
        json!({"type": "mixer.set_volume", "track": a, "gain_db": -6.0}),
    ));
    s.run(cmd(
        json!({"type": "mixer.set_solo", "track": b, "solo": true}),
    ));
    s.run(cmd(json!({"type": "transport.set_tempo", "bpm": 140.0})));
    s.run(cmd(
        json!({"type": "transport.set_metronome", "enabled": true}),
    ));
    s.run(cmd(json!({"type": "transport.play"})));
    s.run(cmd(
        json!({"type": "live.note_on", "track": b, "note": 60, "velocity": 100}),
    ));
    s.render_secs(0.3);
    s.run(cmd(json!({"type": "track.delete", "track": a})));
    s.run(cmd(json!({"type": "history.undo"})));
    s.run(cmd(
        json!({"type": "mixer.set_solo", "track": b, "solo": false}),
    ));
    s.run(cmd(
        json!({"type": "live.note_on", "track": synth, "note": 64, "velocity": 100}),
    ));
    s.render_secs(0.3);
    s.resync();
    s.run(cmd(json!({"type": "transport.play"})));
    s.run(cmd(
        json!({"type": "live.note_on", "track": b, "note": 67, "velocity": 100}),
    ));
    s.render_secs(0.3);
    s.run(cmd(json!({"type": "live.all_notes_off"})));
    s.run(cmd(json!({"type": "transport.stop"})));
    s.render_secs(0.2);

    let (mut prepare, mut host) = engine::host::channel(SAMPLE_RATE).unwrap();
    let mut peak = 0.0f32;
    for step in &s.scenario().steps {
        match step {
            Step::Push(bytes) => prepare.submit(bytes).unwrap(),
            Step::Render(blocks) => {
                for _ in 0..*blocks {
                    prepare.flush();
                    let n = assert_no_alloc(|| host.render(QUANTUM as usize));
                    peak = host.left()[..n].iter().fold(peak, |p, x| p.max(x.abs()));
                }
            }
        }
    }
    prepare.flush();
    assert_eq!(prepare.preparer().rejected(), 0);
    assert_eq!(host.engine().skipped(), 0);
    assert!(peak > 0.05, "the notes and the click sound: {peak}");
}
