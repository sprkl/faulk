//! The cost of the latency compensation (#749) per block, measured in release:
//!
//! `cargo test -p engine --release --test latency_cost -- --nocapture --test-threads=1`
//!
//! Sixty-four playing tracks with no device of latency (the compensation idle: every track
//! straight into the sum, the metronome off), the same with a test delay of 72 frames on the
//! first track (the 63 others through the compensation ring), and 4 096 track slots of which
//! one plays (the latencies read on every slot each block). Printed in microseconds per block
//! of 128 frames at 48 kHz (a budget of 2 667 µs). No allocation detector here: its
//! allocator does not build in release.

use std::time::Instant;

use engine::device::DeviceSetup;
use engine::device::testing::{DELAY, TONE, delay, factory};
use engine::engine::{Engine, Preparer, channel};
use engine::msg::{ControlMsg as M, DeviceSlot, TrackKind, TrackSlot, encode_state};

const BLOCK: usize = 128;

fn rig() -> (Preparer, Engine) {
    channel(
        DeviceSetup {
            sample_rate: 48_000.0,
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
        M::TrackSetMix {
            track: TrackSlot(slot),
            gain: 0.01,
            pan: 0.0,
            mute: false,
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
            at: None,
        },
    ]
}

/// Best of five runs, in microseconds per block.
fn time(e: &mut Engine, blocks: usize) -> f64 {
    let (mut l, mut r) = (vec![0.0; BLOCK], vec![0.0; BLOCK]);
    let mut best = f64::MAX;
    for _ in 0..5 {
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
fn the_cost_of_the_compensation_per_block() {
    let (mut p, mut idle) = rig();
    for slot in 0..64 {
        send(&mut p, &mut idle, &playing(slot));
    }
    let (mut p, mut latent) = rig();
    for slot in 0..64 {
        send(&mut p, &mut latent, &playing(slot));
    }
    send(
        &mut p,
        &mut latent,
        &[M::DeviceInsert {
            track: TrackSlot(0),
            device: DeviceSlot(1),
            kind: DELAY.kind_id,
            state: encode_state(&[(delay::FRAMES, 72.0)]),
        }],
    );
    // The device warms up and its line reaches its latency.
    for _ in 0..8 {
        send(&mut p, &mut latent, &[]);
    }
    let (mut p, mut wide) = rig();
    send(&mut p, &mut wide, &playing(0));
    for slot in 1..4096 {
        p.handle(M::TrackCreate {
            track: TrackSlot(slot),
            kind: TrackKind::Midi,
        })
        .unwrap();
        if slot % 64 == 0 {
            send(&mut p, &mut wide, &[]);
        }
    }
    send(&mut p, &mut wide, &[]);
    assert_eq!(latent.graph().latency(), 72);
    let blocks = 2000;
    let (a, b, c) = (
        time(&mut idle, blocks),
        time(&mut latent, blocks),
        time(&mut wide, blocks / 4),
    );
    eprintln!(
        "per block of {BLOCK} frames: 64 playing tracks, no latency {a:.2} µs; with one device \
         of 72 frames (63 tracks through the ring) {b:.2} µs; 4 096 slots, one playing {c:.2} µs"
    );
}
