//! The cost of the latency compensation (#749) per block, measured in release:
//!
//! `cargo test -p engine --release --test latency_cost -- --nocapture --test-threads=1`
//!
//! Sixty-four playing tracks with no device of latency (the compensation idle: every track
//! straight into the sum, the metronome off), the same with a test delay of 72 frames on the
//! first track (the 63 others through the compensation ring), and 4 096 track slots of which
//! one plays (the latencies read on every slot each block); and 64 playing tracks, each through
//! a test delay of 72 frames then a test gain whose trim a lane moves (#765: the values of a
//! device after a latent one go through that device's event line). Printed in microseconds
//! per block of 128 frames at 48 kHz (a budget of 2 667 µs). No allocation detector here: its
//! allocator does not build in release.

use std::time::Instant;

use engine::device::DeviceSetup;
use engine::device::testing::{DELAY, GAIN, TONE, delay, factory, gain};
use engine::engine::{Engine, Preparer, channel};
use engine::msg::{
    AutomationPoint, AutomationTarget, ControlMsg as M, Curve, DeviceSlot, LaneSlot, TrackKind,
    TrackSlot, encode_state,
};
use engine::time::{Quantize, Tempo, Tick, TimeSignature};

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
    let (mut p, mut lanes) = rig();
    send(
        &mut p,
        &mut lanes,
        &[M::SetTransport {
            tempo: Tempo::new(120.0).unwrap(),
            signature: TimeSignature::COMMON,
            launch_quantize: Quantize::default(),
        }],
    );
    // A slow triangle on the gain's trim: the lane moves (a value every control frame) for
    // the whole measure.
    let points: Vec<AutomationPoint> = (0..256)
        .map(|i| AutomationPoint {
            tick: Tick(i * 960),
            value: if i % 2 == 0 { 0.5 } else { 1.5 },
            curve: Curve::Linear,
        })
        .collect();
    for slot in 0..64u32 {
        let track = TrackSlot(slot);
        let mut msgs = playing(slot);
        msgs.push(M::DeviceInsert {
            track,
            device: DeviceSlot(1),
            kind: DELAY.kind_id,
            state: encode_state(&[(delay::FRAMES, 72.0)]),
        });
        msgs.push(M::DeviceInsert {
            track,
            device: DeviceSlot(2),
            kind: GAIN.kind_id,
            state: encode_state(&[]),
        });
        msgs.push(M::AutomationLaneUpload {
            lane: LaneSlot(slot),
            track,
            target: AutomationTarget::DeviceParam {
                device: DeviceSlot(2),
                param: gain::TRIM,
            },
            first: 0,
            total: points.len() as u32,
            points: points.clone(),
        });
        send(&mut p, &mut lanes, &msgs);
    }
    for _ in 0..8 {
        send(&mut p, &mut lanes, &[]);
    }
    send(&mut p, &mut lanes, &[M::Play { from: Tick(0) }]);
    assert_eq!(latent.graph().latency(), 72);
    assert_eq!(lanes.graph().latency(), 72);
    let blocks = 2000;
    let (a, b, c, d) = (
        time(&mut idle, blocks),
        time(&mut latent, blocks),
        time(&mut wide, blocks / 4),
        time(&mut lanes, blocks),
    );
    eprintln!(
        "per block of {BLOCK} frames: 64 playing tracks, no latency {a:.2} µs; with one device \
         of 72 frames (63 tracks through the ring) {b:.2} µs; 4 096 slots, one playing {c:.2} µs; \
         64 tracks each through a device of 72 frames then a gain a lane moves {d:.2} µs"
    );
}

/// A track through two test delays of 72 frames, then 367 test gains whose 1 100 parameters
/// lanes step at the play: right after it, the first delay's event line is full (#765).
fn full_lines() -> (Preparer, Engine) {
    let (mut p, mut e) = rig();
    send(
        &mut p,
        &mut e,
        &[M::SetTransport {
            tempo: Tempo::new(120.0).unwrap(),
            signature: TimeSignature::COMMON,
            launch_quantize: Quantize::default(),
        }],
    );
    let track = TrackSlot(0);
    let mut msgs = playing(0);
    for d in 1..=2u16 {
        msgs.push(M::DeviceInsert {
            track,
            device: DeviceSlot(d),
            kind: DELAY.kind_id,
            state: encode_state(&[(delay::FRAMES, 72.0)]),
        });
    }
    for d in 0..367u16 {
        msgs.push(M::DeviceInsert {
            track,
            device: DeviceSlot(3 + d),
            kind: GAIN.kind_id,
            state: encode_state(&[]),
        });
    }
    send(&mut p, &mut e, &msgs);
    let lanes: Vec<M> = (0..1_100u32)
        .map(|n| M::AutomationLaneUpload {
            lane: LaneSlot(n),
            track,
            target: AutomationTarget::DeviceParam {
                device: DeviceSlot(3 + (n / 3) as u16),
                param: engine::msg::ParamId(n % 3),
            },
            first: 0,
            total: 2,
            points: vec![
                AutomationPoint {
                    tick: Tick(0),
                    value: if n % 3 == 1 { 0.0 } else { 1.0 },
                    curve: Curve::Step,
                },
                AutomationPoint {
                    tick: Tick(100),
                    value: if n % 3 == 1 { 0.0 } else { 0.9 },
                    curve: Curve::Step,
                },
            ],
        })
        .collect();
    for chunk in lanes.chunks(64) {
        send(&mut p, &mut e, chunk);
    }
    for _ in 0..200 {
        send(&mut p, &mut e, &[]);
    }
    (p, e)
}

/// The block that applies `msgs`, in microseconds, right after a play from tick 100 filled the
/// lines; best of five fresh engines.
fn apply_after_play(msgs: &[M]) -> f64 {
    let mut best = f64::MAX;
    for _ in 0..5 {
        let (mut p, mut e) = full_lines();
        p.handle(M::Play { from: Tick(100) }).unwrap();
        p.flush();
        let (mut l, mut r) = (vec![0.0; BLOCK], vec![0.0; BLOCK]);
        e.process(&mut l, &mut r);
        for m in msgs {
            p.handle(m.clone()).unwrap();
        }
        p.flush();
        let start = Instant::now();
        e.process(&mut l, &mut r);
        best = best.min(start.elapsed().as_secs_f64() * 1e6);
    }
    best
}

#[test]
fn the_cost_of_the_messages_with_full_event_lines() {
    let track = TrackSlot(0);
    let none = apply_after_play(&[]);
    let gestures: Vec<M> = (0..100u16)
        .map(|d| M::ParamSet {
            track,
            device: DeviceSlot(3 + d),
            param: gain::GAIN,
            value: 0.5,
        })
        .collect();
    let set = apply_after_play(&gestures);
    let remove = apply_after_play(&[M::DeviceRemove {
        track,
        device: DeviceSlot(1),
    }]);
    eprintln!(
        "full event lines, the block after the play: {none:.1} µs; with 100 gestures {set:.1} µs \
         ({:.2} µs per gesture); with the first delay removed (its full line handed on) \
         {remove:.1} µs ({:+.1} µs)",
        (set - none) / 100.0,
        remove - none
    );
}

/// 64 playing tracks, each through a test delay of `frames` frames then two test gains; the
/// first one also through a delay of 2 000 frames, so that the other 63 do not hold the
/// longest chain (#761).
fn switching(frames: f32) -> (Preparer, Engine) {
    let (mut p, mut e) = rig();
    for slot in 0..64u32 {
        let track = TrackSlot(slot);
        let mut msgs = playing(slot);
        let delays: &[f32] = if slot == 0 { &[2000.0] } else { &[frames] };
        msgs.push(M::DeviceInsert {
            track,
            device: DeviceSlot(1),
            kind: DELAY.kind_id,
            state: encode_state(&[(delay::FRAMES, delays[0])]),
        });
        for d in 2..4 {
            msgs.push(M::DeviceInsert {
                track,
                device: DeviceSlot(d),
                kind: GAIN.kind_id,
                state: encode_state(&[]),
            });
        }
        send(&mut p, &mut e, &msgs);
    }
    for _ in 0..40 {
        send(&mut p, &mut e, &[]);
    }
    (p, e)
}

#[test]
fn the_cost_of_the_blocks_where_slots_switch() {
    // The 63 shorter tracks lose their delay at once: each fades its device into its line (a
    // fade, the device still running), then catches up `frames` frames (at most a block a
    // block) through its two gains, its strip and the ring. The worst block of the 30 after
    // the removal, against the steady block before it (best of five rigs each).
    for frames in [72.0f32, 1000.0] {
        let (mut steady, mut worst) = (f64::MAX, f64::MAX);
        for _ in 0..5 {
            let (mut p, mut e) = switching(frames);
            steady = steady.min(time(&mut e, 200));
            for slot in 1..64u32 {
                p.handle(M::DeviceRemove {
                    track: TrackSlot(slot),
                    device: DeviceSlot(1),
                })
                .unwrap();
            }
            p.flush();
            let (mut l, mut r) = (vec![0.0; BLOCK], vec![0.0; BLOCK]);
            let mut longest = 0f64;
            for _ in 0..30 {
                let start = Instant::now();
                e.process(&mut l, &mut r);
                longest = longest.max(start.elapsed().as_secs_f64() * 1e6);
                p.collect();
            }
            worst = worst.min(longest);
            assert_eq!(e.graph().latency(), 2000);
        }
        eprintln!(
            "63 tracks lose a delay of {frames} frames at once: steady {steady:.1} µs a block, \
             the worst block after {worst:.1} µs"
        );
    }
}
