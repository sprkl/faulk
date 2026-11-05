//! #560: a take recorded round the transport's loop, from the commands to the engine and back.
//! `record.start` gives the engine the take's shift (the latency the host estimates); the
//! engine moves each note back by it on its own clock, where it knows the loop; the notes it
//! reports are written by `record.commit_take` with a shift of 0, as the page does. Stamps are
//! engine samples here (the native host without a host clock).

use commands::{Command, Output};
use engine::msg::{ControlMsg as M, RecordedNote, TrackSlot};
use project::{ClipContent, TrackKind};
use serde_json::json;
use test_support::{QUANTUM, Scenario, Session, Step};

/// 48 kHz at 120 BPM: 25 samples a tick, 24 000 a beat, 96 000 a bar.
const SPT: f64 = 25.0;
const BEAT: u64 = 24_000;
const BAR: u64 = 4 * BEAT;
/// The estimated output latency: 42 ms, 2 016 samples.
const LATENCY: u64 = 2_016;
/// The loop: the second bar.
const LOOP_START: i64 = 3840;
const LOOP_END: i64 = 7680;
/// +-1 ms at 120 BPM.
const TOLERANCE: f64 = 1.92;

fn command(value: serde_json::Value) -> Command {
    serde_json::from_value(value).expect("a command")
}

/// Replays the scenario on the native host and returns the notes the engine recorded.
fn recorded(scenario: &Scenario) -> Vec<RecordedNote> {
    let (prepare, mut host) = engine::host::channel(48_000.0).expect("the channel opens");
    let mut queue = engine::host::StreamQueue::new(prepare);
    for step in &scenario.steps {
        match step {
            Step::Push(bytes) => queue.send(bytes).expect("the stream is accepted"),
            Step::Render(blocks) => {
                for _ in 0..*blocks {
                    queue.flush().expect("the stream is accepted");
                    host.render(QUANTUM as usize);
                }
            }
        }
    }
    assert_eq!(host.engine().skipped(), 0);
    let mut out = vec![];
    host.engine().recorder().recent(|_, n| out.push(*n));
    out
}

#[test]
fn a_take_round_the_loop_lands_where_each_note_was_heard_within_a_millisecond() {
    let mut s = Session::single_synth();
    let track = s
        .state()
        .project
        .tracks
        .iter()
        .find(|(_, t)| t.kind == TrackKind::Midi)
        .map(|(id, _)| id.clone())
        .expect("a MIDI track");
    s.run(command(json!({"type": "transport.set_metronome", "count_in_bars": 0})))
        .run(command(json!({"type": "transport.set_loop", "loop_region": {"start": LOOP_START, "end": LOOP_END}, "enabled": true})))
        .run(command(json!({"type": "track.arm", "track": track, "armed": true})))
        .render_blocks(10);
    // The take plays from tick 0 at the next block: sample 1 280.
    let zero = 10 * QUANTUM as u64;
    let Output::Record(commands::record::Output::Start { shift_ms, .. }) =
        s.exec(command(json!({"type": "record.start", "latency_ms": 42.0})))
    else {
        panic!("record.start")
    };
    assert_eq!(shift_ms, 42.0);
    let mut rendered = zero;
    // Each note: heard at `heard`, stamped one latency later, taken three blocks after that.
    let mut play = |s: &mut Session, pitch: u8, heard: u64| {
        let stamp = heard + LATENCY;
        let until = stamp + 3 * QUANTUM as u64;
        let blocks = until.div_ceil(QUANTUM as u64) - rendered / QUANTUM as u64;
        s.render_blocks(blocks as u32);
        rendered += blocks * QUANTUM as u64;
        s.messages(&[M::NoteOn {
            track: TrackSlot(0),
            pitch,
            velocity: 100,
            at: Some(stamp),
        }])
        .render_blocks(1);
        rendered += QUANTUM as u64;
        s.messages(&[M::NoteOff {
            track: TrackSlot(0),
            pitch,
            at: Some(stamp + 240),
        }]);
    };
    // On beat 1, so that the take's clip starts on bar 0 and holds every note where it lands.
    play(&mut s, 60, zero + BEAT);
    // 10 ms before the playhead enters the loop (its first pass): stamped inside the loop.
    play(&mut s, 62, zero + BAR - 480);
    // 20 ms before the loop's end: stamped after the wrap.
    play(&mut s, 64, zero + 2 * BAR - 960);
    s.render_blocks(2)
        .run(command(json!({"type": "record.stop"})))
        .render_blocks(2);
    let notes = recorded(s.scenario());
    assert_eq!(notes.len(), 3, "{notes:?}");
    let tracks = json!([{
        "track": track,
        "notes": notes.iter().map(|n| json!({
            "start": n.note.start.0,
            "length": n.note.length.0,
            "pitch": n.note.pitch,
            "velocity": n.note.velocity,
        })).collect::<Vec<_>>(),
    }]);
    let Output::Record(commands::record::Output::CommitTake { clips, dropped, .. }) = s.exec(
        command(json!({"type": "record.commit_take", "take": 1, "target": {"at": "arrangement"}, "shift_ms": 0.0, "tracks": tracks})),
    ) else {
        panic!("record.commit_take")
    };
    assert_eq!((clips.len(), dropped), (1, 0), "{clips:?}");
    let t = &s.state().project.tracks[&track];
    let placement = t
        .placements
        .values()
        .find(|p| p.clip == clips[0].clip)
        .expect("the take's placement");
    let ClipContent::Midi { notes, .. } = &t.clips[&clips[0].clip].content else {
        panic!("a MIDI clip")
    };
    let at = |pitch: u8| {
        placement.start.get()
            + notes
                .iter()
                .find(|n| n.pitch == pitch)
                .expect("the note")
                .start
                .get()
    };
    let heard = |samples_from_zero: u64| samples_from_zero as f64 / SPT;
    for (pitch, expected) in [
        (60, heard(BEAT)),
        (62, heard(BAR - 480)),
        (64, heard(2 * BAR - 960)),
    ] {
        let landed = at(pitch) as f64;
        assert!(
            (landed - expected).abs() <= TOLERANCE,
            "note {pitch} at {landed}, heard at {expected}"
        );
    }
    assert!(
        at(62) < LOOP_START,
        "before the loop, not folded to its end"
    );
    assert!(at(64) < LOOP_END && at(64) > LOOP_END - 81);
}
