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

/// CL-3: what a `ClipPut` over a playing clip adds to `apply`, at the table's limit: the
/// search of the sounding notes in the new table and the aliases (`carry_held_from`).
#[test]
#[ignore = "prints numbers, run in release by hand"]
fn clip_swap_added_cost_at_the_limit() {
    use engine::clip_player::{ClipNotes, MAX_CARRIED_NOTES};
    use engine::events::{Aliases, Events};
    let count = limits::MAX_NOTES_PER_CLIP as usize;
    let rounds = 50;
    let run = |label: &str, notes: &[Note], held: &[usize], shift: bool| {
        let mut total = 0f64;
        for _ in 0..rounds {
            let mut old = ClipNotes::try_new(notes).unwrap();
            for &i in held {
                old.held_bits_mut()[i / 32] |= 1 << (i % 32);
            }
            let mut moved: Vec<Note> = Vec::new();
            if shift {
                moved.push(Note {
                    start: Tick(0),
                    length: Tick(1),
                    pitch: 1,
                    ..Note::default()
                });
                moved.extend_from_slice(notes);
            }
            let mut new = ClipNotes::try_new(if shift { &moved } else { notes }).unwrap();
            let t0 = Instant::now();
            let mut orphans = 0;
            let (before, mut now) = (Aliases::default(), Aliases::default());
            let carried = new.carry_held_from(&old, &before, &mut now, |_| orphans += 1);
            total += t0.elapsed().as_nanos() as f64;
            assert_eq!(carried as usize + orphans, held.len());
        }
        println!(
            "{label}: {:.1} us (mean of {rounds})",
            total / f64::from(rounds) / 1000.0
        );
    };
    let spread: Vec<Note> = (0..count)
        .map(|i| Note {
            start: Tick(i as i64),
            length: Tick(100),
            pitch: (i % 128) as u8,
            velocity: 100,
            ..Note::default()
        })
        .collect();
    let few: Vec<usize> = (0..8).map(|k| 1000 + k).collect();
    let many: Vec<usize> = (0..MAX_CARRIED_NOTES).map(|k| 1000 + 3 * k).collect();
    run("1M notes, 8 held, same table", &spread, &few, false);
    run("1M notes, 8 held, a note added before", &spread, &few, true);
    run(
        "1M notes, 256 held, a note added before",
        &spread,
        &many,
        true,
    );
    // The worst case of the scan: every note on one tick.
    let crowd: Vec<Note> = (0..count)
        .map(|i| Note {
            start: Tick(0),
            length: Tick(100),
            pitch: (i % 128) as u8,
            velocity: 100,
            ..Note::default()
        })
        .collect();
    let crowd_held: Vec<usize> = (0..MAX_CARRIED_NOTES).map(|k| 500_000 + k).collect();
    run("1M notes on one tick, 256 held", &crowd, &crowd_held, true);
    let _ = ClipNotes::try_new(&[]).unwrap().len();
}
