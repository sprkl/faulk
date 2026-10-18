//! Native cost of the engine channel, on this machine (CT-3's measure, redone on MO-1's
//! real `Preparer` and `Engine`): per rt message (prepare and push of a `ParamSet`, then
//! pop and apply on the audio side), and per growth (the `apply` that swaps a full runtime
//! table for one twice as large: the track table at its limit, the clip directory at its
//! limit, a device row at its limit; and the drop of the old table on the prepare side).
//! Ignored by default; run with:
//!
//! `cargo test -p engine --release --test msg_cost -- --ignored --nocapture`

use std::time::Instant;

use engine::device::DeviceSetup;
use engine::device::testing::{GAIN, factory, gain};
use engine::engine::{Engine, Preparer, channel};
use engine::msg::{
    ClipSlot, ControlMsg as M, DeviceSlot, Note, PAGE_SLOTS, TrackKind, TrackSlot, limits,
};
use engine::time::Tick;

fn create(slot: u32) -> M {
    M::TrackCreate {
        track: TrackSlot(slot),
        kind: TrackKind::Midi,
    }
}

/// Prepares `msg`, then times its `apply` alone (µs) and the drop of its garbage (µs).
fn timed(p: &mut Preparer, e: &mut Engine, msg: M) -> (f64, f64) {
    let rt = p.prepare(msg).unwrap().unwrap();
    let t0 = Instant::now();
    e.apply(rt);
    let t1 = Instant::now();
    p.collect();
    let t2 = Instant::now();
    (
        (t1 - t0).as_nanos() as f64 / 1000.0,
        (t2 - t1).as_nanos() as f64 / 1000.0,
    )
}

#[test]
#[ignore = "prints numbers, run in release by hand"]
fn per_message_and_per_growth_cost() {
    let (mut p, mut e) = channel(DeviceSetup::default(), factory).unwrap();
    p.handle(create(0)).unwrap();
    p.handle(M::DeviceInsert {
        track: TrackSlot(0),
        device: DeviceSlot(0),
        kind: GAIN.kind_id,
        state: vec![],
    })
    .unwrap();
    e.begin_block();

    // Per message: the prepare side fills the ring with ParamSets, the audio side drains it.
    let rounds = 200;
    let n = limits::RT_RING_CAPACITY;
    let (mut push_ns, mut apply_ns) = (0f64, 0f64);
    for round in 0..rounds {
        let t0 = Instant::now();
        for i in 0..n {
            p.handle(M::ParamSet {
                track: TrackSlot(0),
                device: DeviceSlot(0),
                param: gain::GAIN,
                value: ((i + round) % 100) as f32 / 100.0,
            })
            .unwrap();
        }
        let t1 = Instant::now();
        while e.applied() < ((round + 1) * n + 2) as u64 {
            e.begin_block();
        }
        let t2 = Instant::now();
        push_ns += (t1 - t0).as_nanos() as f64 / n as f64;
        apply_ns += (t2 - t1).as_nanos() as f64 / n as f64;
    }
    println!(
        "per message: prepare + push {:.0} ns, pop + apply {:.0} ns (ParamSet, {rounds} rounds of {n})",
        push_ns / rounds as f64,
        apply_ns / rounds as f64
    );

    // Per growth of the track table: 64, 1024 and 2048 tracks (the last doubles to the
    // limit, 4096), each track with a device.
    for tracks in [64u32, 1024, limits::MAX_TRACKS / 2] {
        let (mut p, mut e) = channel(DeviceSetup::default(), factory).unwrap();
        for slot in 0..tracks {
            let rt = p.prepare(create(slot)).unwrap().unwrap();
            e.apply(rt);
            p.collect();
        }
        assert_eq!(e.state().tracks.capacity(), tracks as usize);
        let (apply, drop) = timed(&mut p, &mut e, create(tracks));
        println!(
            "growth of the track table, {tracks} tracks moved: apply {apply:.1} µs, drop off-thread {drop:.1} µs"
        );
        assert_eq!(e.state().tracks.capacity(), 2 * tracks as usize);
    }

    // Per growth of the clip directory to its limit: one pointer per page.
    let (mut p, mut e) = channel(DeviceSetup::default(), factory).unwrap();
    let upload = |clip: u32| M::ClipNotesUpload {
        clip: ClipSlot(clip),
        length: Tick(960),
        looping: None,
        first: 0,
        total: 1,
        notes: vec![Note::default()],
    };
    let half = limits::MAX_CLIPS / 2;
    for page in 0..half / PAGE_SLOTS as u32 {
        let rt = p
            .prepare(upload(page * PAGE_SLOTS as u32))
            .unwrap()
            .unwrap();
        e.apply(rt);
        p.collect();
    }
    let (apply, drop) = timed(&mut p, &mut e, upload(half));
    println!(
        "growth of the clip directory, {} pages moved ({} clips addressed): apply {apply:.1} µs, drop {drop:.1} µs",
        half / PAGE_SLOTS as u32,
        half
    );
    assert_eq!(e.state().clips.capacity(), limits::MAX_CLIPS as usize);

    // A device row to its limit.
    let (mut p, mut e) = channel(DeviceSetup::default(), factory).unwrap();
    p.handle(create(0)).unwrap();
    e.begin_block();
    let devices = limits::MAX_DEVICES_PER_TRACK / 2;
    for d in 0..devices {
        let rt = p
            .prepare(M::DeviceInsert {
                track: TrackSlot(0),
                device: DeviceSlot(d as u16),
                kind: GAIN.kind_id,
                state: vec![],
            })
            .unwrap()
            .unwrap();
        e.apply(rt);
        p.collect();
    }
    let (apply, drop) = timed(
        &mut p,
        &mut e,
        M::DeviceInsert {
            track: TrackSlot(0),
            device: DeviceSlot(devices as u16),
            kind: GAIN.kind_id,
            state: vec![],
        },
    );
    println!(
        "growth of a device row, {devices} devices moved: apply {apply:.1} µs, drop {drop:.1} µs"
    );
}
