//! The paced send of the native hosts (#245, `engine::host::StreamQueue`): a stream longer than
//! the engine takes at once (rt ring plus prepare queue) is sent in pieces as the engine
//! consumes it, in order, and the render never allocates meanwhile. Every state transition of
//! the queue: through at once, busy and kept, a later stream behind a waiting one, a stream
//! stopped for another reason, the bound.

use assert_no_alloc::assert_no_alloc;
use engine::host::{Host, MAX_WAITING_BYTES, StreamQueue, channel};
use engine::msg::{ControlMsg as M, MsgError, TrackKind, TrackSlot, encode_stream, limits};

const RATE: f32 = 48_000.0;
/// More rt messages than the ring and the prepare queue hold together.
const BIG: usize = limits::RT_RING_CAPACITY + limits::PENDING_QUEUE_CAPACITY + 1_000;

struct Rig {
    queue: StreamQueue,
    host: Host,
    seq: u32,
}

impl Rig {
    fn new() -> Self {
        Self::bounded(MAX_WAITING_BYTES)
    }

    fn bounded(max_waiting: usize) -> Self {
        let (prepare, host) = channel(RATE).unwrap();
        let mut rig = Rig {
            queue: StreamQueue::with_max_waiting(prepare, max_waiting),
            host,
            seq: 0,
        };
        let reset = rig.encode(&[M::Reset { epoch: 1 }, create(0)]);
        rig.queue.send(&reset).unwrap();
        rig
    }

    fn encode(&mut self, msgs: &[M]) -> Vec<u8> {
        let bytes = encode_stream(1, self.seq, msgs).unwrap();
        self.seq += msgs.len() as u32;
        bytes
    }

    /// One block of the audio thread (under the allocation detector), then the control
    /// thread's tick.
    fn block(&mut self) -> Result<(), MsgError> {
        let host = &mut self.host;
        assert_no_alloc(|| host.render(128));
        self.queue.flush()
    }

    /// Blocks until nothing waits nor is pending; how many.
    fn drain(&mut self) -> usize {
        let mut blocks = 0;
        while self.queue.waiting() > 0 || self.queue.prepare().preparer().pending() > 0 {
            self.block().unwrap();
            blocks += 1;
            assert!(blocks < 10_000, "the queue does not drain");
        }
        // What the last flush pushed onto the ring.
        for _ in 0..limits::RT_RING_CAPACITY.div_ceil(limits::MAX_APPLY_PER_BLOCK) {
            self.block().unwrap();
        }
        blocks
    }
}

fn create(slot: u32) -> M {
    M::TrackCreate {
        track: TrackSlot(slot),
        kind: TrackKind::Midi,
    }
}

fn solo(slot: u32, solo: bool) -> M {
    M::TrackSetSolo {
        track: TrackSlot(slot),
        solo,
    }
}

/// `n` solo toggles of track 0, ending soloed when `n` is odd.
fn toggles(n: usize) -> Vec<M> {
    (0..n).map(|i| solo(0, i % 2 == 0)).collect()
}

#[test]
fn a_stream_that_fits_goes_through_at_once() {
    let mut rig = Rig::new();
    let bytes = rig.encode(&toggles(10));
    rig.queue.send(&bytes).unwrap();
    assert_eq!(rig.queue.waiting(), 0);
    assert_eq!(rig.queue.waiting_bytes(), 0);
    rig.drain();
    assert_eq!(rig.host.engine().applied(), 2 + 10);
}

#[test]
fn a_stream_longer_than_the_engine_takes_is_sent_in_pieces_in_order() {
    let mut rig = Rig::new();
    let big = rig.encode(&toggles(BIG));
    rig.queue.send(&big).unwrap();
    assert_eq!(rig.queue.waiting(), 1, "the rest waits for the engine");
    assert!(rig.queue.waiting_bytes() > 0 && rig.queue.waiting_bytes() < big.len());
    // Sent while the first waits: it goes behind it. Its solo depends on its own create,
    // which depends on nothing; the solo of track 0 comes last, after every toggle.
    let later = rig.encode(&[create(5), solo(5, true), solo(0, false)]);
    rig.queue.send(&later).unwrap();
    assert_eq!(rig.queue.waiting(), 2);
    let blocks = rig.drain();
    assert!(blocks > 1, "taken in several blocks: {blocks}");
    assert_eq!(rig.queue.waiting_bytes(), 0);
    let engine = rig.host.engine();
    assert_eq!(engine.applied() as usize, 2 + BIG + 3, "nothing lost");
    assert_eq!(engine.skipped(), 0, "nothing overtook what it depends on");
    assert_eq!(rig.queue.prepare().preparer().rejected(), 0);
    assert!(engine.state().track(TrackSlot(5)).unwrap().solo);
    assert!(
        !engine.state().track(TrackSlot(0)).unwrap().solo,
        "the later stream's solo came after the big one's last toggle"
    );
}

#[test]
fn an_engine_that_does_not_render_keeps_the_rest_until_it_does() {
    // The switch back to Web pushes its snapshot into a suspended audio context.
    let mut rig = Rig::new();
    let big = rig.encode(&toggles(BIG));
    rig.queue.send(&big).unwrap();
    let waiting = rig.queue.waiting_bytes();
    for _ in 0..50 {
        rig.queue.flush().unwrap();
    }
    assert_eq!(
        rig.queue.waiting_bytes(),
        waiting,
        "nothing taken while suspended"
    );
    rig.drain();
    assert_eq!(rig.host.engine().applied() as usize, 2 + BIG);
    let last = rig.host.engine().state().track(TrackSlot(0)).unwrap().solo;
    assert_eq!(last, BIG % 2 == 1, "the last toggle is the one that stays");
}

#[test]
fn a_stream_stopped_for_another_reason_is_dropped_and_the_next_goes_on() {
    let mut rig = Rig::new();
    let big = rig.encode(&toggles(BIG));
    rig.queue.send(&big).unwrap();
    // A corrupt stream behind it: a message, then a header cut short.
    let mut corrupt = rig.encode(&[create(3)]);
    corrupt.extend_from_slice(&[1, 2, 3]);
    rig.queue.send(&corrupt).unwrap();
    let after = rig.encode(&[create(4)]);
    rig.queue.send(&after).unwrap();
    assert_eq!(rig.queue.waiting(), 3);
    let mut errors = Vec::new();
    while rig.queue.waiting() > 0 {
        if let Err(error) = rig.block() {
            errors.push(error);
        }
    }
    rig.drain();
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(
        matches!(errors[0], MsgError::Truncated { .. }),
        "{errors:?}"
    );
    let state = rig.host.engine().state();
    assert!(
        state.track(TrackSlot(3)).is_some(),
        "the message before the stop was handled"
    );
    assert!(
        state.track(TrackSlot(4)).is_some(),
        "the next stream went through"
    );
    assert_eq!(rig.queue.waiting_bytes(), 0);
}

#[test]
fn a_corrupt_stream_with_nothing_waiting_is_returned_at_once() {
    let mut rig = Rig::new();
    let mut corrupt = rig.encode(&[create(3)]);
    corrupt.extend_from_slice(&[1, 2, 3]);
    assert!(matches!(
        rig.queue.send(&corrupt),
        Err(MsgError::Truncated { .. })
    ));
    assert_eq!(rig.queue.waiting(), 0);
}

#[test]
fn past_the_bound_a_later_stream_is_refused_whole_and_the_front_one_never() {
    let bound = 16 * 1024;
    let mut rig = Rig::bounded(bound);
    // The front stream is never refused, even when its rest is over the bound: a snapshot
    // larger than the bound is what a host sends to recover.
    let big = rig.encode(&toggles(BIG));
    rig.queue.send(&big).unwrap();
    assert!(rig.queue.waiting_bytes() > bound);
    // Behind it, within the bound: kept.
    let fits = rig.encode(&toggles(101));
    rig.queue.send(&fits).unwrap();
    assert_eq!(rig.queue.waiting(), 2);
    // A stream that would take what waits behind the front one past the bound: refused,
    // nothing of it handled nor queued.
    let over = rig.encode(&vec![create(7); bound / 16]);
    assert!(over.len() > bound);
    assert_eq!(rig.queue.send(&over), Err(MsgError::EngineBusy));
    assert_eq!(rig.queue.waiting(), 2, "the refused stream is not queued");
    rig.drain();
    let engine = rig.host.engine();
    assert_eq!(engine.applied() as usize, 2 + BIG + 101);
    assert!(engine.state().track(TrackSlot(7)).is_none());
    // Once drained, a stream is taken again.
    let small = rig.encode(&[create(6)]);
    rig.queue.send(&small).unwrap();
    rig.drain();
    assert!(rig.host.engine().state().track(TrackSlot(6)).is_some());
    assert_eq!(rig.host.engine().skipped(), 0);
}

#[test]
fn clear_drops_what_waits_and_the_next_stream_goes_straight_in() {
    let mut rig = Rig::new();
    let big = rig.encode(&toggles(BIG));
    rig.queue.send(&big).unwrap();
    let later = rig.encode(&[create(8)]);
    rig.queue.send(&later).unwrap();
    assert_eq!(rig.queue.waiting(), 2);
    rig.queue.clear();
    assert_eq!((rig.queue.waiting(), rig.queue.waiting_bytes()), (0, 0));
    rig.drain();
    let taken = rig.host.engine().applied() as usize - 2;
    assert!(
        taken < BIG,
        "only what the ring and the queue held: {taken}"
    );
    assert!(rig.host.engine().state().track(TrackSlot(8)).is_none());
    let next = rig.encode(&[create(9)]);
    rig.queue.send(&next).unwrap();
    assert_eq!(rig.queue.waiting(), 0);
    rig.drain();
    assert!(rig.host.engine().state().track(TrackSlot(9)).is_some());
}

#[test]
fn a_stream_that_starts_with_a_reset_drops_what_waits() {
    // `project.open` or a resync while a snapshot still goes in: the old one is not applied
    // in full only to be wiped.
    let mut rig = Rig::new();
    let big = rig.encode(&toggles(BIG));
    rig.queue.send(&big).unwrap();
    let stale = rig.encode(&[create(8)]);
    rig.queue.send(&stale).unwrap();
    assert_eq!(rig.queue.waiting(), 2);
    let reset = encode_stream(2, 0, &[M::Reset { epoch: 2 }, create(9)]).unwrap();
    rig.queue.send(&reset).unwrap();
    // What waited is gone; the reset itself waits for the room the old messages hold.
    assert_eq!(rig.queue.waiting(), 1);
    assert!(rig.queue.waiting_bytes() <= reset.len());
    rig.drain();
    let engine = rig.host.engine();
    assert!(engine.state().track(TrackSlot(9)).is_some());
    assert!(engine.state().track(TrackSlot(8)).is_none());
    assert_eq!(rig.queue.prepare().preparer().epoch(), 2);
    assert_eq!(engine.skipped(), 0);
}
