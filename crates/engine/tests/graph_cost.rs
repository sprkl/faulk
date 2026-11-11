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
            at: None,
        },
    ]
}

/// Best of `rounds` runs per engine, in microseconds per block, the engines measured in
/// turn inside every round: a stretch of machine load (a parallel build) hits the three of
/// them alike and each one's best round is one the load spared, so the figures compared
/// below share the machine's state instead of each one getting its own (#763).
fn time_all(engines: &mut [&mut Engine], blocks: usize, rounds: usize) -> Vec<f64> {
    let (mut l, mut r) = (vec![0.0; BLOCK], vec![0.0; BLOCK]);
    let mut best = vec![f64::MAX; engines.len()];
    for _ in 0..rounds {
        for (e, best) in engines.iter_mut().zip(&mut best) {
            for _ in 0..20 {
                e.process(&mut l, &mut r);
            }
            let start = Instant::now();
            for _ in 0..blocks {
                e.process(&mut l, &mut r);
            }
            *best = best.min(start.elapsed().as_secs_f64() * 1e6 / blocks as f64);
        }
    }
    best
}

/// Best of three runs of one engine, in microseconds per block.
fn time(e: &mut Engine, blocks: usize) -> f64 {
    time_all(&mut [e], blocks, 3)[0]
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

    let times = time_all(&mut [&mut base, &mut empties, &mut many], blocks, 15);
    let (b, e, p) = (times[0], times[1], times[2]);
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
    // An empty track costs a slot visit (about 1/40 of a playing track here); one that
    // rendered its graph would cost a good fraction of a playing one. The bound sits between
    // the two, on a ratio measured on the same rounds, so the machine's load cancels out.
    // Debug builds have no tighter figure: a looser bound there.
    let ratio = if cfg!(debug_assertions) { 10.0 } else { 20.0 };
    assert!(
        per_empty * ratio < per_playing,
        "an empty track costs {per_empty} µs, a playing one {per_playing} µs"
    );
    // No bound on `share` itself: a share of a wall-clock budget moves with the machine's
    // load (#763). The ratio above, measured on the same rounds, is the check; the share is
    // printed for the record (MO-3).
}

/// The cost of the device fades (#233) in their worst case: a chain of `OUTGOING_ROOM`
/// filters after the instrument, every one of them replaced at once (what a device inserted
/// at the head of such a chain does through the commands), so that for one fade every slot
/// runs its old device and its new one and blends them. Printed per block against the same
/// chain settled; the fade lasts 240 frames, so the two blocks after the change are timed.
#[test]
fn a_whole_chain_rebuilt_at_once_costs_one_fade_of_double_work() {
    use engine::devices::filter::DESCRIPTOR as FILTER;
    use engine::graph::OUTGOING_ROOM;
    let filter = |slot: usize| M::DeviceInsert {
        track: TrackSlot(0),
        device: DeviceSlot(slot as u16),
        kind: FILTER.kind_id,
        state: encode_state(&[]),
    };
    let (mut p, mut e) = rig();
    let mut setup = playing(0);
    setup.extend((1..=OUTGOING_ROOM).map(filter));
    send(&mut p, &mut e, &setup);
    let steady = time(&mut e, 300);
    let (mut l, mut r) = (vec![0.0; BLOCK], vec![0.0; BLOCK]);
    let rounds = 100;
    let mut best = f64::MAX;
    for _ in 0..3 {
        let mut total = 0.0;
        for _ in 0..rounds {
            let rts: Vec<_> = (1..=OUTGOING_ROOM)
                .map(|slot| p.prepare(filter(slot)).unwrap().unwrap())
                .collect();
            for rt in rts {
                e.apply(rt);
            }
            assert_eq!(
                e.state().track(TrackSlot(0)).unwrap().graph.outgoing_len(),
                OUTGOING_ROOM
            );
            let start = Instant::now();
            for _ in 0..2 {
                e.render(&mut l, &mut r);
            }
            total += start.elapsed().as_secs_f64() * 1e6 / 2.0;
            // The outgoing filters left at the end of the fade; drop them and the garbage.
            assert_eq!(
                e.state().track(TrackSlot(0)).unwrap().graph.outgoing_len(),
                0
            );
            p.collect();
        }
        best = best.min(total / f64::from(rounds));
    }
    eprintln!(
        "per block of {BLOCK} frames ({BUDGET_US:.0} µs budget), a tone and {OUTGOING_ROOM} \
         filters: settled {steady:.2} µs; every filter replaced, the two blocks of the fade \
         {best:.2} µs ({:.2}x)",
        best / steady
    );
    if !cfg!(debug_assertions) {
        assert!(
            best < BUDGET_US / 2.0,
            "the fade of a whole chain takes {best:.2} µs of a {BUDGET_US:.0} µs block"
        );
    }
}
