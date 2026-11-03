//! The cost of automation playback (AM-2): ten tracks, each a test tone through the test
//! gain effect, rendered while playing without lanes, then with 100 lanes of 1 000 points
//! (ten per track: gain, pan and eight device parameters, half of the transitions linear).
//! Printed per 128-frame block at 48 kHz (budget 2 667 µs). Measured in release with:
//!
//! `cargo test -p engine --release --test automation_cost -- --nocapture`
//!
//! (no allocation detector here: its allocator does not build in release; the allocation
//! test is `tests/it/automation.rs`).

use std::time::Instant;

use engine::device::DeviceSetup;
use engine::device::testing::{GAIN, TONE, factory, gain, tone};
use engine::engine::{Engine, Preparer, channel};
use engine::msg::{
    AutomationPoint, AutomationTarget, ControlMsg as M, Curve, DeviceSlot, LaneSlot, TrackKind,
    TrackSlot, encode_state,
};
use engine::time::{Quantize, Tempo, Tick, TimeSignature};

const BLOCK: usize = 128;
const RATE: f32 = 48_000.0;
const BUDGET_US: f64 = BLOCK as f64 / RATE as f64 * 1e6;
const TRACKS: u32 = 10;

fn send(p: &mut Preparer, e: &mut Engine, msgs: &[M]) {
    for m in msgs {
        p.handle(m.clone()).unwrap();
    }
    p.flush();
    let (mut l, mut r) = (vec![0.0; BLOCK], vec![0.0; BLOCK]);
    e.process(&mut l, &mut r);
    p.collect();
}

fn rig(lanes: bool) -> (Preparer, Engine) {
    let (mut p, mut e) = channel(
        DeviceSetup {
            sample_rate: RATE,
            max_block_frames: BLOCK,
        },
        factory,
    )
    .unwrap();
    send(
        &mut p,
        &mut e,
        &[M::SetTransport {
            tempo: Tempo::new(120.0).unwrap(),
            signature: TimeSignature::COMMON,
            launch_quantize: Quantize::default(),
        }],
    );
    let mut slot = 0;
    for t in 0..TRACKS {
        let track = TrackSlot(t);
        send(
            &mut p,
            &mut e,
            &[
                M::TrackCreate {
                    track,
                    kind: TrackKind::Midi,
                },
                M::DeviceInsert {
                    track,
                    device: DeviceSlot(0),
                    kind: TONE.kind_id,
                    state: encode_state(&[]),
                },
                M::DeviceInsert {
                    track,
                    device: DeviceSlot(1),
                    kind: GAIN.kind_id,
                    state: encode_state(&[]),
                },
                M::NoteOn {
                    track,
                    pitch: 60 + t as u8,
                    velocity: 100,
                    at: None,
                },
            ],
        );
        if !lanes {
            continue;
        }
        for k in 0..10u32 {
            let target = match k {
                0 => AutomationTarget::MixerGain,
                1 => AutomationTarget::MixerPan,
                2..=4 => AutomationTarget::DeviceParam {
                    device: DeviceSlot(1),
                    param: gain::TRIM,
                },
                5..=7 => AutomationTarget::DeviceParam {
                    device: DeviceSlot(1),
                    param: gain::GAIN,
                },
                _ => AutomationTarget::DeviceParam {
                    device: DeviceSlot(0),
                    param: tone::GAIN,
                },
            };
            let points: Vec<AutomationPoint> = (0..1_000i64)
                .map(|i| AutomationPoint {
                    tick: Tick(i * 40 + i64::from(k)),
                    value: match target {
                        AutomationTarget::MixerGain => -20.0 + (i % 9) as f32,
                        AutomationTarget::MixerPan => (i % 5) as f32 / 5.0 - 0.4,
                        _ => 0.2 + (i % 7) as f32 * 0.1,
                    },
                    curve: if i % 2 == 0 {
                        Curve::Linear
                    } else {
                        Curve::Step
                    },
                })
                .collect();
            send(
                &mut p,
                &mut e,
                &[M::AutomationLaneUpload {
                    lane: LaneSlot(slot),
                    track,
                    target,
                    first: 0,
                    total: points.len() as u32,
                    points,
                }],
            );
            slot += 1;
        }
    }
    send(&mut p, &mut e, &[M::Play { from: Tick(0) }]);
    (p, e)
}

/// Best of five runs, in microseconds per block.
fn time(e: &mut Engine, blocks: usize) -> f64 {
    let (mut l, mut r) = (vec![0.0; BLOCK], vec![0.0; BLOCK]);
    let mut best = f64::MAX;
    for _ in 0..5 {
        let start = Instant::now();
        for _ in 0..blocks {
            e.process(&mut l, &mut r);
        }
        best = best.min(start.elapsed().as_secs_f64() * 1e6 / blocks as f64);
    }
    best
}

#[test]
#[ignore = "a measure: run it in release with --nocapture"]
fn a_hundred_lanes_cost_little_of_a_block() {
    // 750 blocks: two seconds, a bar at 120 BPM (each run goes on from the last).
    let blocks = 750;
    let (_p0, mut bare) = rig(false);
    let (_p1, mut lanes) = rig(true);
    assert_eq!(lanes.state().lanes.iter().count(), 100);
    let b = time(&mut bare, blocks);
    let l = time(&mut lanes, blocks);
    let extra = (l - b).max(0.0);
    eprintln!(
        "per block of {BLOCK} frames ({BUDGET_US:.0} µs budget): {TRACKS} playing tracks \
         {b:.2} µs; with 100 lanes of 1 000 points {l:.2} µs (+{extra:.2} µs, {:.2} % of the \
         budget)",
        extra / BUDGET_US * 100.0
    );
}

/// #378: the ten tracks again, with the test gain effect in the master's chain and, when
/// `lanes`, ten lanes of 1 000 points on the master (its gain and pan, and eight parameters
/// of the effect), half of the transitions linear.
fn master_rig(lanes: bool) -> (Preparer, Engine) {
    let (p, mut e) = rig(false);
    let mut p = p;
    send(
        &mut p,
        &mut e,
        &[M::DeviceInsert {
            track: TrackSlot::MASTER,
            device: DeviceSlot(0),
            kind: GAIN.kind_id,
            state: encode_state(&[]),
        }],
    );
    if !lanes {
        return (p, e);
    }
    for k in 0..10u32 {
        let target = match k {
            0 => AutomationTarget::MixerGain,
            1 => AutomationTarget::MixerPan,
            2..=5 => AutomationTarget::DeviceParam {
                device: DeviceSlot(0),
                param: gain::TRIM,
            },
            _ => AutomationTarget::DeviceParam {
                device: DeviceSlot(0),
                param: gain::GAIN,
            },
        };
        let points: Vec<AutomationPoint> = (0..1_000i64)
            .map(|i| AutomationPoint {
                tick: Tick(i * 40 + i64::from(k)),
                value: match target {
                    AutomationTarget::MixerGain => -20.0 + (i % 9) as f32,
                    AutomationTarget::MixerPan => (i % 5) as f32 / 5.0 - 0.4,
                    _ => 0.2 + (i % 7) as f32 * 0.1,
                },
                curve: if i % 2 == 0 {
                    Curve::Linear
                } else {
                    Curve::Step
                },
            })
            .collect();
        send(
            &mut p,
            &mut e,
            &[M::AutomationLaneUpload {
                lane: LaneSlot(k),
                track: TrackSlot::MASTER,
                target,
                first: 0,
                total: points.len() as u32,
                points,
            }],
        );
    }
    (p, e)
}

#[test]
#[ignore = "a measure: run it in release with --nocapture"]
fn ten_master_lanes_cost_little_of_a_block() {
    let blocks = 750;
    let (_p0, mut bare) = master_rig(false);
    let (_p1, mut lanes) = master_rig(true);
    assert_eq!(lanes.state().lanes.iter().count(), 10);
    let b = time(&mut bare, blocks);
    let l = time(&mut lanes, blocks);
    let extra = (l - b).max(0.0);
    eprintln!(
        "per block of {BLOCK} frames ({BUDGET_US:.0} µs budget): {TRACKS} playing tracks and one \
         master effect {b:.2} µs; with 10 master lanes of 1 000 points {l:.2} µs (+{extra:.2} µs, \
         {:.2} % of the budget)",
        extra / BUDGET_US * 100.0
    );
}
