//! The cost of the timed events (MO-4): a block of 2 048 frames at 48 kHz that carries
//! 10 000 notes (20 000 events) from a schedule to the test instrument, cut in sub-blocks
//! whenever the sink is full. Printed against the block's budget. Measured in release with:
//!
//! `cargo test -p engine --release --test events_cost -- --nocapture`
//!
//! (no allocation detector here: its allocator does not build in release; `events.rs`
//! renders the same load under it).

use std::time::Instant;

use engine::device::DeviceSetup;
use engine::device::testing::{TONE, factory};
use engine::events::{SCHEDULE_FIRST_VOICE, Schedule, ScheduledNote};
use engine::msg::{ControlMsg as M, DeviceSlot, TrackKind, TrackSlot};
use engine::time::Tick;

const BLOCK: usize = 2048;
const RATE: f32 = 48_000.0;

#[test]
fn cost_of_ten_thousand_notes_in_a_block() {
    let (mut p, mut e) = engine::engine::channel(
        DeviceSetup {
            sample_rate: RATE,
            max_block_frames: BLOCK,
        },
        factory,
    )
    .unwrap();
    let track = TrackSlot(0);
    let send = |p: &mut engine::engine::Preparer, msgs: &[M]| {
        for m in msgs {
            p.handle(m.clone()).unwrap();
        }
        p.flush();
    };
    send(
        &mut p,
        &[
            M::TrackCreate {
                track,
                kind: TrackKind::Midi,
            },
            M::DeviceInsert {
                track,
                device: DeviceSlot(0),
                kind: TONE.kind_id,
                state: Vec::new(),
            },
        ],
    );
    let (mut l, mut r) = (vec![0.0f32; BLOCK], vec![0.0f32; BLOCK]);
    e.process(&mut l, &mut r);
    // 120 BPM at 48 kHz: 25 samples per tick, the block holds about 82 ticks.
    let notes: Vec<ScheduledNote> = (0..10_000)
        .map(|i| ScheduledNote {
            start: Tick(2 + (i % 80)),
            length: Tick(1),
            key: 60,
            velocity: 1.0,
            tune_cents: 0.0,
        })
        .collect();
    let mut runs = Vec::new();
    for _ in 0..30 {
        let s = Schedule::try_new(&notes, &[], SCHEDULE_FIRST_VOICE).unwrap();
        drop(
            e.state_mut()
                .track_mut(track)
                .unwrap()
                .graph
                .set_schedule(Some(Box::new(s))),
        );
        send(&mut p, &[M::Play { from: Tick(0) }]);
        let t = Instant::now();
        e.process(&mut l, &mut r);
        runs.push(t.elapsed().as_secs_f64() * 1e6);
        send(&mut p, &[M::Stop]);
        e.process(&mut l, &mut r);
        p.collect();
    }
    runs.sort_by(f64::total_cmp);
    let budget = BLOCK as f64 / f64::from(RATE) * 1e6;
    println!(
        "10 000 notes in a block of {BLOCK} frames: median {:.0} µs, max {:.0} µs, budget {budget:.0} µs",
        runs[runs.len() / 2],
        runs[runs.len() - 1],
    );
}
