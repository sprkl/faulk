//! The cost of the arrangement (AR-2) in a block of 2 048 frames at 48 kHz, printed against
//! the block's budget (42.7 ms). Two loads: one track whose arrangement holds a placement
//! of one tick on every tick (a note each: the player changes placement about 82 times per
//! block), and 1 000 tracks that each hold one long placement of a clip with no note (the cost of
//! a track that has nothing to deliver, against the same engine without placements). Measured in release with:
//!
//! `cargo test -p engine --release --test arrangement_cost -- --nocapture`
//!
//! (no allocation detector here: its allocator does not build in release; `arrangement.rs`
//! renders the same projects under it). The assertions hold in release only, with a budget
//! of a quarter of the block.

use std::time::Instant;

use engine::device::DeviceSetup;
use engine::device::testing::{TONE, factory};
use engine::engine::{Engine, Preparer, channel};
use engine::msg::{
    ClipSlot, ControlMsg as M, DeviceSlot, Note, Placement, PlacementSlot, Span, TrackKind,
    TrackSlot,
};
use engine::time::Tick;

const BLOCK: usize = 2048;
const RATE: f32 = 48_000.0;
const BLOCKS: usize = 400;

fn rig(tracks: u32) -> (Preparer, Engine) {
    let (mut p, e) = channel(
        DeviceSetup {
            sample_rate: RATE,
            max_block_frames: BLOCK,
        },
        factory,
    )
    .unwrap();
    for t in 0..tracks {
        for m in [
            M::TrackCreate {
                track: TrackSlot(t),
                kind: TrackKind::Midi,
            },
            M::DeviceInsert {
                track: TrackSlot(t),
                device: DeviceSlot(0),
                kind: TONE.kind_id,
                state: Vec::new(),
            },
        ] {
            p.handle(m).unwrap();
        }
    }
    p.flush();
    (p, e)
}

fn placement(slot: u32, clip: u32, start: i64, length: i64) -> Placement {
    Placement {
        placement: PlacementSlot(slot),
        clip: ClipSlot(clip),
        start: Tick(start),
        span: Span::Ticks {
            offset: Tick(0),
            length: Tick(length),
        },
    }
}

fn note(start: i64, length: i64) -> Note {
    Note {
        start: Tick(start),
        length: Tick(length),
        pitch: 60,
        velocity: 100,
        ..Note::default()
    }
}

/// Renders `BLOCKS` blocks from tick 0 and returns the microseconds per block.
fn per_block(p: &mut Preparer, e: &mut Engine) -> f64 {
    p.handle(M::Play { from: Tick(0) }).unwrap();
    p.flush();
    let (mut l, mut r) = (vec![0.0f32; BLOCK], vec![0.0f32; BLOCK]);
    e.process(&mut l, &mut r);
    let start = Instant::now();
    for _ in 0..BLOCKS {
        e.process(&mut l, &mut r);
        p.collect();
    }
    start.elapsed().as_secs_f64() * 1e6 / BLOCKS as f64
}

fn check(what: &str, micros: f64) {
    let budget = BLOCK as f64 / f64::from(RATE) * 1e6;
    println!("{what}: {micros:.1} µs per block of {BLOCK} frames (the block lasts {budget:.0} µs)");
    if !cfg!(debug_assertions) {
        assert!(micros < budget / 4.0, "{what}: {micros} µs");
    }
}

#[test]
fn a_placement_on_every_tick() {
    let (mut p, mut e) = rig(1);
    // 100 000 placements of one tick, the clip a one-tick note: 82 changes per block.
    let notes = vec![note(0, 1)];
    p.handle(M::ClipNotesUpload {
        clip: ClipSlot(0),
        length: Tick(1),
        looping: None,
        first: 0,
        total: 1,
        notes,
    })
    .unwrap();
    let placements: Vec<Placement> = (0..100_000)
        .map(|i| placement(i, 0, i64::from(i), 1))
        .collect();
    p.handle(M::TrackPlacementsUpload {
        track: TrackSlot(0),
        first: 0,
        total: placements.len() as u32,
        placements,
    })
    .unwrap();
    p.flush();
    check("one placement per tick", per_block(&mut p, &mut e));
}

#[test]
fn a_thousand_tracks_with_one_long_placement_of_a_silent_clip() {
    // The visit of the player to a track that has nothing to deliver, apart from the cost
    // of the devices (the same engine without placements is the baseline).
    let (mut p, mut e) = rig(1000);
    let baseline = per_block(&mut p, &mut e);
    let (mut p, mut e) = rig(1000);
    for t in 0..1000 {
        p.handle(M::ClipNotesUpload {
            clip: ClipSlot(t),
            length: Tick(1_000_000),
            looping: None,
            first: 0,
            total: 0,
            notes: Vec::new(),
        })
        .unwrap();
        p.handle(M::TrackPlacementsUpload {
            track: TrackSlot(t),
            first: 0,
            total: 1,
            placements: vec![placement(t, t, 0, 1_000_000)],
        })
        .unwrap();
    }
    p.flush();
    let with = per_block(&mut p, &mut e);
    println!("1 000 tracks without placements (the baseline): {baseline:.1} µs per block");
    check("1 000 tracks, one placement each", with);
    println!(
        "the arrangement costs {:.1} µs per block here",
        with - baseline
    );
}
