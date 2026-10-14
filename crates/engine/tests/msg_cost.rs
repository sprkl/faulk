//! Native cost of the engine channel (CT-3), on this machine: per rt message (push, pop,
//! apply of a `ParamSet`) and per growth (the `apply` of a `Swap` that moves a full track
//! table of boxed entries, and the drop of the old table on the control side). Ignored by
//! default; run with:
//!
//! `cargo test -p engine --release --test msg_cost -- --ignored --nocapture`

use std::time::Instant;

use engine::msg::{
    DeviceSlot, Garbage, Opaque, ParamId, Prepared, RtMsg, Swap, Table, TableKind, TrackSlot,
    limits,
};
use rtrb::RingBuffer;

#[test]
#[ignore = "prints numbers, run in release by hand"]
fn per_message_and_per_growth_cost() {
    let (mut rt_tx, mut rt_rx) = RingBuffer::<RtMsg>::new(limits::RT_RING_CAPACITY);
    let (mut gc_tx, mut gc_rx) = RingBuffer::<Garbage>::new(limits::RT_RING_CAPACITY);

    // Per message: fill the ring, drain it applying each message to a parameter slot.
    let mut params = vec![0f32; 64];
    let rounds = 200;
    let n = limits::RT_RING_CAPACITY;
    let mut push_ns = 0f64;
    let mut pop_ns = 0f64;
    for _ in 0..rounds {
        let t0 = Instant::now();
        for i in 0..n {
            rt_tx
                .push(RtMsg::ParamSet {
                    track: TrackSlot(0),
                    device: DeviceSlot(0),
                    param: ParamId((i % 64) as u32),
                    value: i as f32,
                    offset: 0,
                })
                .unwrap();
        }
        let t1 = Instant::now();
        while let Ok(msg) = rt_rx.pop() {
            if let RtMsg::ParamSet { param, value, .. } = msg {
                params[param.0 as usize] = value;
            }
        }
        let t2 = Instant::now();
        push_ns += (t1 - t0).as_nanos() as f64 / n as f64;
        pop_ns += (t2 - t1).as_nanos() as f64 / n as f64;
    }
    println!(
        "per message: push {:.0} ns, pop + apply {:.0} ns (ring of {n}, {rounds} rounds)",
        push_ns / rounds as f64,
        pop_ns / rounds as f64
    );
    assert!(params[1] > 0.0);

    // Per growth: a full track table of MAX_TRACKS boxed entries moved into one twice as
    // large, in apply; then the drop of the emptied table on the control side.
    for tracks in [64usize, 1024, limits::MAX_TRACKS as usize] {
        let mut audio: Table<Option<Opaque>> = Table::try_with_capacity(tracks).unwrap();
        for i in 0..tracks {
            audio.push(Some(Box::new([i as u8; 256]))).unwrap();
        }
        let next: Table<Option<Opaque>> = Table::try_with_capacity(tracks * 2).unwrap();
        rt_tx
            .push(RtMsg::Swap(Swap {
                kind: TableKind::Tracks,
                next: Prepared::Tracks(next),
            }))
            .unwrap();
        let t0 = Instant::now();
        let RtMsg::Swap(Swap {
            next: Prepared::Tracks(mut next),
            ..
        }) = rt_rx.pop().unwrap()
        else {
            unreachable!()
        };
        next.take_from(&mut audio);
        let old = core::mem::replace(&mut audio, next);
        gc_tx.push(Garbage::Tracks(old)).unwrap();
        let t1 = Instant::now();
        drop(gc_rx.pop().unwrap());
        let t2 = Instant::now();
        println!(
            "per growth of the track table, {tracks} entries: apply {:.1} µs, drop off-thread {:.1} µs",
            (t1 - t0).as_nanos() as f64 / 1000.0,
            (t2 - t1).as_nanos() as f64 / 1000.0
        );
        assert_eq!(audio.len(), tracks);
        assert_eq!(audio.capacity(), tracks * 2);
    }

    // The counter-example that fixes the rule "content tables are snapshots": moving
    // a million notes in apply, as a growth in place would.
    let n = 1_000_000;
    let mut notes: Table<engine::msg::Note> = Table::try_with_capacity(n).unwrap();
    for _ in 0..n {
        notes.push(engine::msg::Note::default()).unwrap();
    }
    let mut next: Table<engine::msg::Note> = Table::try_with_capacity(n * 2).unwrap();
    let t0 = Instant::now();
    next.take_from(&mut notes);
    let t1 = Instant::now();
    println!(
        "moving {n} notes in apply would cost {:.0} µs (hence content tables are swapped by pointer)",
        (t1 - t0).as_nanos() as f64 / 1000.0
    );
    assert_eq!(next.len(), n);
}
