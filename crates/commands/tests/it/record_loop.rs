//! #542: `record.commit_take` moves a take back by the output latency. A note heard in the last
//! milliseconds of the transport's loop is stamped by the engine just after the loop's start
//! (`crates/engine/tests/it/live_stamps.rs`, `a_note_heard_just_before_the_loop_end_is_placed_after_the_wrap_by_the_latency`),
//! so the move back must follow the loop, not the line.
//! Since #560 the page's takes are moved by the engine, which knows the wraps, and come with a
//! shift of 0 (`crates/test-support/tests/it/record_loop.rs`): this fold serves an explicit shift.

use crate::record_examples;
use ::project::{Tick, TrackId};
use commands::record::{Command as R, Output as RO, RecordTarget};
use commands::testing::context;
use commands::{Command, Output, State};
use record_examples::{Rec, note, rec, take};

/// The loop: bars 2 and 3 at 4/4 and 960 ticks per quarter (3 840 to 11 520).
const LOOP_START: i64 = 3840;
const LOOP_END: i64 = 11_520;
/// At 120 BPM a millisecond is 1.92 ticks; 42 ms are 81 ticks (80.64 rounded).
const SHIFT_MS: f64 = 42.0;
const SHIFT: i64 = 81;
/// +-1 ms.
const TOLERANCE: i64 = 2;

fn looping(r: &Rec, enabled: bool) -> State {
    let mut state = r.state.clone();
    state.project.transport.loop_region =
        engine::time::LoopRegion::new(Tick(LOOP_START), Tick(LOOP_END));
    state.project.transport.loop_enabled = enabled;
    state
}

/// Commits notes stamped at `stamps` (transport ticks) to the arrangement with the 42 ms shift,
/// and returns the transport tick each lands at (the placement's start plus the note's start).
fn landed(r: &Rec, state: &State, stamps: &[i64], shift_ms: f64) -> Vec<i64> {
    let mut state = state.clone();
    let tracks = take(r, stamps.iter().map(|s| note(*s, 60, 240)).collect(), None);
    let outcome = commands::run(
        &Command::Record(R::CommitTake {
            replace: None,
            quantize_on_input: None,
            quantize_strength: None,
            take: None,
            target: RecordTarget::Arrangement {},
            shift_ms,
            tracks,
        }),
        &mut state,
        &mut context(),
    )
    .unwrap();
    state.project.validate().unwrap();
    let Output::Record(RO::CommitTake { clips, dropped, .. }) = outcome.output else {
        panic!()
    };
    assert_eq!(dropped, 0);
    let track: &TrackId = &r.track;
    let mut ticks = vec![];
    for c in &clips {
        let placement = state.project.tracks[track]
            .placements
            .values()
            .find(|p| p.clip == c.clip)
            .unwrap();
        let clip = &state.project.tracks[track].clips[&c.clip];
        let ::project::ClipContent::Midi { notes, .. } = &clip.content else {
            panic!()
        };
        ticks.extend(notes.iter().map(|n| placement.start.get() + n.start.get()));
    }
    ticks.sort_unstable();
    ticks
}

/// The tick the host hears a note at, `ms` before the loop's end.
fn heard(ms_before_end: f64) -> i64 {
    LOOP_END - (ms_before_end * 1.92).round() as i64
}

/// The tick the engine stamps a note heard `ms` before the loop's end: its heard tick plus the
/// latency, wrapped into the loop.
fn stamped(ms_before_end: f64) -> i64 {
    let t = heard(ms_before_end) + SHIFT;
    if t >= LOOP_END {
        LOOP_START + (t - LOOP_END)
    } else {
        t
    }
}

#[test]
fn a_note_heard_20_ms_before_the_loop_end_is_recorded_there_within_a_millisecond() {
    let r = rec();
    // The engine test pins this premise: heard 20 ms before the end, stamped 42 ticks into the loop.
    assert!((stamped(20.0) - (LOOP_START + 42)).abs() <= 1);
    let ticks = landed(
        &r,
        &looping(&r, true),
        &[stamped(60.0), stamped(20.0)],
        SHIFT_MS,
    );
    assert!((ticks[0] - heard(60.0)).abs() <= TOLERANCE, "{ticks:?}");
    assert!(
        (ticks[1] - heard(20.0)).abs() <= TOLERANCE,
        "{ticks:?} against {}",
        heard(20.0)
    );
}

#[test]
fn the_move_follows_the_line_without_a_loop_or_outside_it() {
    let r = rec();
    // A transport that does not loop (the region is kept, the loop is off): the line, as before.
    let off = looping(&r, false);
    assert_eq!(
        landed(&r, &off, &[LOOP_START + 500], SHIFT_MS),
        vec![LOOP_START + 500 - SHIFT]
    );
    // A loop, and a stamp that the move keeps inside it.
    let on = looping(&r, true);
    assert_eq!(
        landed(&r, &on, &[LOOP_START + 500], SHIFT_MS),
        vec![LOOP_START + 500 - SHIFT]
    );
    // A stamp outside the loop (after its end) moves on the line.
    assert_eq!(
        landed(&r, &on, &[LOOP_END + 100], SHIFT_MS),
        vec![LOOP_END + 100 - SHIFT]
    );
    // A stamp exactly on the loop's start (a note pressed before the capture, clamped to the take's
    // first tick) and a negative shift (a late offset) stay on the line (the first one then goes to the first beat of its clip, the early rule).
    assert_eq!(landed(&r, &on, &[LOOP_START], SHIFT_MS), vec![LOOP_START]);
    assert_eq!(
        landed(&r, &on, &[LOOP_END - 10], -SHIFT_MS),
        vec![LOOP_END - 10 + SHIFT]
    );
    // No shift, no change, even right at the loop's start.
    assert_eq!(
        landed(&r, &on, &[LOOP_START + 10], 0.0),
        vec![LOOP_START + 10]
    );
}

#[test]
fn a_shift_longer_than_the_loop_wraps_into_it() {
    let r = rec();
    // A one-beat loop (960 ticks) and a 600 ms shift (1 152 ticks, a loop and a fifth): a note
    // stamped 10 ticks into the loop was heard 1 142 ticks earlier, 182 ticks before the loop's end.
    let mut state = r.state.clone();
    state.project.transport.loop_region =
        engine::time::LoopRegion::new(Tick(LOOP_START), Tick(LOOP_START + 960));
    state.project.transport.loop_enabled = true;
    let ticks = landed(&r, &state, &[LOOP_START + 10], 600.0);
    assert_eq!(ticks, vec![LOOP_START + 960 - 182]);
}
