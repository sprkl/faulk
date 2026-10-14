//! The cost of an empty track in the graph (MO-3: "une piste vide ne coûte pas de calcul
//! notable, mesuré"). An empty track is a track with no enabled device: the graph visits
//! its slot and renders nothing. Printed per track and as a share of the budget of a
//! 128-frame block at 48 kHz (2.67 ms). Measured in release with:
//!
//! `cargo test -p engine --release --test graph_cost -- --nocapture`
//!
//! (no allocation detector here: its allocator does not build in release).

use std::time::Instant;

use engine::device::DeviceSetup;
use engine::device::testing::{TONE, factory};
use engine::engine::{Engine, Preparer, channel};
use engine::msg::{ControlMsg as M, DeviceSlot, TrackKind, TrackSlot, encode_state};

const BLOCK: usize = 128;
const RATE: f32 = 48_000.0;
/// Microseconds of a 128-frame block at 48 kHz.
const BUDGET_US: f64 = BLOCK as f64 / RATE as f64 * 1e6;

fn rig() -> (Preparer, Engine) {
    channel(
        DeviceSetup {
            sample_rate: RATE,
            max_block_frames: BLOCK,
        },
        factory,
    )
    .unwrap()
}

fn send(p: &mut Preparer, e: &mut Engine, msgs: &[M]) {
    for m in msgs {
        p.handle(m.clone()).unwrap();
    }
    p.flush();
    let (mut l, mut r) = (vec![0.0; BLOCK], vec![0.0; BLOCK]);
    e.process(&mut l, &mut r);
}

fn playing(slot: u32) -> Vec<M> {
    vec![
        M::TrackCreate {
            track: TrackSlot(slot),
            kind: TrackKind::Midi,
        },
        M::DeviceInsert {
            track: TrackSlot(slot),
            device: DeviceSlot(0),
            kind: TONE.kind_id,
            state: encode_state(&[]),
        },
        M::NoteOn {
            track: TrackSlot(slot),
            pitch: 69,
            velocity: 100,
        },
    ]
}

/// Best of three runs, in microseconds per block.
fn time(e: &mut Engine, blocks: usize) -> f64 {
    let (mut l, mut r) = (vec![0.0; BLOCK], vec![0.0; BLOCK]);
    let mut best = f64::MAX;
    for _ in 0..3 {
        for _ in 0..20 {
            e.process(&mut l, &mut r);
        }
        let start = Instant::now();
        for _ in 0..blocks {
            e.process(&mut l, &mut r);
        }
        best = best.min(start.elapsed().as_secs_f64() * 1e6 / blocks as f64);
    }
    best
}

#[test]
fn an_empty_track_costs_no_notable_computation() {
    const PLAYING: u32 = 32;
    // Every other slot of the track table at its limit (4 096) is empty.
    const EMPTY: u32 = 4_095;
    let blocks = 300;

    let (mut pb, mut base) = rig();
    send(&mut pb, &mut base, &playing(0));
    let (mut pe, mut empties) = rig();
    send(&mut pe, &mut empties, &playing(0));
    for slot in 1..=EMPTY {
        pe.handle(M::TrackCreate {
            track: TrackSlot(slot),
            kind: TrackKind::Midi,
        })
        .unwrap();
        if slot % 64 == 0 {
            send(&mut pe, &mut empties, &[]);
        }
    }
    send(&mut pe, &mut empties, &[]);
    assert_eq!(
        empties.state().tracks.as_slice().iter().flatten().count(),
        EMPTY as usize + 1
    );
    let (mut pp, mut many) = rig();
    for slot in 0..=PLAYING {
        send(&mut pp, &mut many, &playing(slot));
    }

    let b = time(&mut base, blocks);
    let e = time(&mut empties, blocks);
    let p = time(&mut many, blocks);
    let per_empty = (e - b).max(0.0) / f64::from(EMPTY);
    let per_playing = (p - b).max(0.0) / f64::from(PLAYING);
    let share = (e - b).max(0.0) / BUDGET_US * 100.0;
    eprintln!(
        "per block of {BLOCK} frames ({BUDGET_US:.0} µs budget): 1 playing track {b:.2} µs; \
         + {EMPTY} empty tracks {e:.2} µs ({:.1} ns each, {share:.2} % of the budget for all \
         of them); + {PLAYING} playing tracks {p:.2} µs ({:.0} ns each)",
        per_empty * 1e3,
        per_playing * 1e3,
    );
    // Debug builds share the machine with the other tests: a looser bound there.
    let ratio = if cfg!(debug_assertions) { 10.0 } else { 50.0 };
    assert!(
        per_empty * ratio < per_playing,
        "an empty track costs {per_empty} µs, a playing one {per_playing} µs"
    );
    if !cfg!(debug_assertions) {
        assert!(
            share < 5.0,
            "{EMPTY} empty tracks take {share:.2} % of a block's budget"
        );
    }
}
