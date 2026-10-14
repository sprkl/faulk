//! Native prototype of the prepare/apply cycle of `engine::msg` (CT-3): a control side that
//! builds tables and an audio side that swaps them under the allocation detector, through
//! every state transition of the cycle, and the attack cases the issue lists. MO-1 turns
//! this into `Engine::prepare` and `Engine::apply`; the contract is what these tests pin.

use std::collections::VecDeque;

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use engine::msg::{
    ClipSlot, ControlMsg, Garbage, HEADER_BYTES, Header, LiveOverflow, MSG_VERSION, MsgError, Note,
    Opaque, Prepared, RtMsg, Swap, Table, TableFull, TableKind, Tag, TrackSlot, limits, try_box,
};
use engine::time::Tick;
use rtrb::{Consumer, Producer, RingBuffer};

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

const CLIP: ClipSlot = ClipSlot(0);

/// A track as MO-3 will box it: it owns its devices, so removing it is one garbage item.
struct TrackState {
    #[allow(dead_code)]
    devices: Vec<Opaque>,
}

/// The audio side: the live tables (the track table is a slot array) and the two rings'
/// ends it holds.
struct AudioSide {
    tracks: Table<Option<Opaque>>,
    notes: Table<Note>,
    rt: Consumer<RtMsg>,
    garbage: Producer<Garbage>,
    applied: u32,
    /// Messages naming a slot beyond the table: a contract violation, counted, no panic.
    skipped: u32,
    rendered_blocks: u32,
    overflow_seen: u32,
    releases: u32,
}

impl AudioSide {
    /// `Engine::apply` of MO-1 for the swaps and removes: no allocation, no free.
    fn apply_pending(&mut self) {
        for _ in 0..limits::MAX_APPLY_PER_BLOCK {
            let Ok(msg) = self.rt.pop() else { break };
            self.applied += 1;
            let garbage = match msg {
                RtMsg::Swap(Swap {
                    kind: TableKind::ClipNotes(_),
                    next: Prepared::ClipNotes(next),
                }) => Garbage::ClipNotes(core::mem::replace(&mut self.notes, next)),
                RtMsg::Swap(Swap {
                    kind: TableKind::Tracks,
                    next: Prepared::Tracks(mut next),
                }) => {
                    next.take_slots_from(&mut self.tracks);
                    Garbage::Tracks(core::mem::replace(&mut self.tracks, next))
                }
                RtMsg::Swap(Swap {
                    kind: TableKind::Track(slot),
                    next: Prepared::Track(track),
                }) => match self.tracks.as_mut_slice().get_mut(slot.0 as usize) {
                    Some(entry) => match entry.replace(track) {
                        Some(old) => Garbage::Track(old),
                        None => continue,
                    },
                    None => {
                        // Beyond the capacity: skipped, and the boxed track goes to the
                        // garbage like anything else apply does not keep.
                        self.skipped += 1;
                        Garbage::Track(track)
                    }
                },
                RtMsg::Remove(TableKind::Track(slot)) => {
                    match self.tracks.as_mut_slice().get_mut(slot.0 as usize) {
                        Some(entry) => match entry.take() {
                            Some(old) => Garbage::Track(old),
                            None => continue,
                        },
                        None => {
                            self.skipped += 1;
                            continue;
                        }
                    }
                }
                _ => unreachable!("this prototype swaps clip notes and tracks only"),
            };
            // The invariant (garbage ring drained before each push) makes this slot free.
            assert!(
                self.garbage.push(garbage).is_ok(),
                "the garbage ring must never be full in apply"
            );
        }
    }

    /// One block: apply what arrived (the live queue first, in the real engine), then the
    /// overflow check and its release, then "render".
    fn render_block(&mut self, overflow: &LiveOverflow) {
        self.apply_pending();
        let total = overflow.total();
        if total != self.overflow_seen {
            self.overflow_seen = total;
            self.releases += 1; // every live note released, after the queued events
        }
        self.rendered_blocks += 1;
    }

    fn filled(&self) -> usize {
        self.tracks
            .as_slice()
            .iter()
            .filter(|e| e.is_some())
            .count()
    }
}

/// The control side: prepares, queues what the ring refuses, collects the garbage.
struct ControlSide {
    rt: Producer<RtMsg>,
    garbage: Consumer<Garbage>,
    /// What the full ring refused, in order; pushed as soon as a slot frees.
    pending: VecDeque<RtMsg>,
    pending_capacity: usize,
    /// The capacity of the track table it last gave the audio side.
    track_capacity: usize,
    collected: u32,
    busy: u32,
}

impl ControlSide {
    /// Drains the garbage ring: the drops happen here, never on the audio side.
    fn collect(&mut self) {
        while let Ok(garbage) = self.garbage.pop() {
            drop(garbage);
            self.collected += 1;
        }
    }

    /// Sends an rt message: through the ring, or into the ordered queue when it is full;
    /// `EngineBusy` once the queue is full too (the engine is not consuming).
    fn send(&mut self, msg: RtMsg) -> Result<(), MsgError> {
        self.flush();
        if !self.pending.is_empty() || self.rt.is_full() {
            if self.pending.len() >= self.pending_capacity {
                return Err(MsgError::EngineBusy);
            }
            self.busy += 1;
            self.pending.push_back(msg);
            return Ok(());
        }
        if let Err(rtrb::PushError::Full(refused)) = self.rt.push(msg) {
            self.busy += 1;
            self.pending.push_back(refused);
        }
        Ok(())
    }

    /// Pushes what waited, in order, after draining the garbage (the invariant holds for
    /// every push, queued or not).
    fn flush(&mut self) {
        self.collect();
        while let Some(msg) = self.pending.pop_front() {
            if let Err(rtrb::PushError::Full(refused)) = self.rt.push(msg) {
                self.pending.push_front(refused);
                break;
            }
        }
    }

    /// `Engine::prepare` for a notes upload: a new snapshot.
    fn prepare_notes(&mut self, notes: &[Note]) -> Result<(), MsgError> {
        let mut next = Table::<Note>::try_with_capacity(notes.len())?;
        for n in notes {
            let _ = next.push(*n);
        }
        self.send(RtMsg::Swap(Swap {
            kind: TableKind::ClipNotes(CLIP),
            next: Prepared::ClipNotes(next),
        }))
    }

    /// `Engine::prepare` for a track table too small: the next slot array, twice as large.
    fn prepare_grow_tracks(&mut self, next_capacity: usize) -> Result<(), MsgError> {
        let next = Table::<Option<Opaque>>::try_filled(next_capacity)?;
        self.track_capacity = next_capacity;
        self.send(RtMsg::Swap(Swap {
            kind: TableKind::Tracks,
            next: Prepared::Tracks(next),
        }))
    }

    /// `Engine::prepare` for a track: grows the table first when the slot is beyond it.
    fn prepare_track(&mut self, slot: TrackSlot, devices: usize) -> Result<(), MsgError> {
        while slot.0 as usize >= self.track_capacity {
            let next = (self.track_capacity * 2).max(1);
            self.prepare_grow_tracks(next)?;
        }
        let mut state = TrackState {
            devices: Vec::new(),
        };
        for _ in 0..devices {
            state.devices.push(try_box([0u8; 64])? as Opaque);
        }
        self.send(RtMsg::Swap(Swap {
            kind: TableKind::Track(slot),
            next: Prepared::Track(try_box(state)?),
        }))
    }
}

fn sides(ring_capacity: usize, table_capacity: usize) -> (ControlSide, AudioSide) {
    let (rt_tx, rt_rx) = RingBuffer::new(ring_capacity);
    // One slot more than the rt ring (the invariant of the module doc).
    let (gc_tx, gc_rx) = RingBuffer::new(ring_capacity + 1);
    (
        ControlSide {
            rt: rt_tx,
            garbage: gc_rx,
            pending: VecDeque::new(),
            pending_capacity: limits::PENDING_QUEUE_CAPACITY,
            track_capacity: table_capacity,
            collected: 0,
            busy: 0,
        },
        AudioSide {
            tracks: Table::try_filled(table_capacity).unwrap(),
            notes: Table::try_with_capacity(0).unwrap(),
            rt: rt_rx,
            garbage: gc_tx,
            applied: 0,
            skipped: 0,
            rendered_blocks: 0,
            overflow_seen: 0,
            releases: 0,
        },
    )
}

fn note(i: usize) -> Note {
    Note {
        start: Tick(i as i64 * 240),
        length: Tick(240),
        pitch: (i % 128) as u8,
        velocity: 100,
    }
}

#[test]
fn a_runtime_table_goes_through_every_transition_without_allocating_on_the_audio_side() {
    let (mut control, mut audio) = sides(limits::RT_RING_CAPACITY, 2);
    let overflow = LiveOverflow::new();

    // A slot array: every slot exists, empty.
    assert_eq!(audio.tracks.len(), 2);
    assert_eq!(audio.filled(), 0);
    // empty → filling → full: two tracks fill the two slots, with no allocation in apply.
    control.prepare_track(TrackSlot(0), 2).unwrap();
    control.prepare_track(TrackSlot(1), 0).unwrap();
    assert_no_alloc(|| audio.render_block(&overflow));
    assert_eq!(audio.filled(), 2);
    assert_eq!(audio.tracks.capacity(), 2);

    // full → growing: a track in slot 2 makes prepare build the next table first (this
    // allocates, off the audio side); nothing changes before apply.
    control.prepare_track(TrackSlot(2), 1).unwrap();
    assert_eq!(control.track_capacity, 4);
    assert_eq!(audio.tracks.capacity(), 2, "nothing changes before apply");

    // growing → swapped: apply moves the boxed entries (one pointer each) into the first
    // slots of the new array, swaps, then lands the third track, with no allocation and
    // no free.
    assert_no_alloc(|| audio.render_block(&overflow));
    assert_eq!(audio.applied, 4);
    assert_eq!(audio.tracks.capacity(), 4);
    assert_eq!(audio.filled(), 3);
    assert_eq!(audio.skipped, 0);

    // swapped → collected: the old (emptied) table is freed on the control side.
    assert_eq!(control.collected, 0);
    control.collect();
    assert_eq!(control.collected, 1);
}

#[test]
fn a_slot_beyond_the_table_is_skipped_and_counted_never_a_panic() {
    // A contract violation (the project named a slot prepare did not grow for): the
    // message is skipped, its box goes to the garbage, the audio side keeps rendering.
    let (mut control, mut audio) = sides(8, 2);
    let overflow = LiveOverflow::new();
    control
        .send(RtMsg::Swap(Swap {
            kind: TableKind::Track(TrackSlot(5)),
            next: Prepared::Track(try_box([0u8; 16]).unwrap() as Opaque),
        }))
        .unwrap();
    control
        .send(RtMsg::Remove(TableKind::Track(TrackSlot(7))))
        .unwrap();
    assert_no_alloc(|| audio.render_block(&overflow));
    assert_eq!(audio.applied, 2);
    assert_eq!(audio.skipped, 2);
    assert_eq!(audio.filled(), 0);
    control.collect();
    assert_eq!(
        control.collected, 1,
        "the refused box was freed off the audio path"
    );
}

#[test]
fn a_content_table_is_a_snapshot_swapped_by_pointer_whatever_its_size() {
    let (mut control, mut audio) = sides(8, 1);
    let overflow = LiveOverflow::new();
    let big: Vec<Note> = (0..100_000).map(note).collect();
    control.prepare_notes(&big).unwrap();
    assert_no_alloc(|| audio.render_block(&overflow));
    assert_eq!(audio.notes.len(), 100_000);
    assert_eq!(audio.notes.as_slice()[99_999], note(99_999));
    // An edit is a new snapshot; the old one is one garbage item, dropped off the audio path.
    control.prepare_notes(&big[..10]).unwrap();
    assert_no_alloc(|| audio.render_block(&overflow));
    assert_eq!(audio.notes.len(), 10);
    control.collect();
    assert_eq!(control.collected, 2);
}

#[test]
fn growth_during_playback_loses_nothing_and_skips_no_block() {
    let (mut control, mut audio) = sides(16, 1);
    let overflow = LiveOverflow::new();
    for slot in 0..600u32 {
        // The project keeps creating tracks while the audio side renders every block.
        control.prepare_track(TrackSlot(slot), 1).unwrap();
        assert_no_alloc(|| audio.render_block(&overflow));
    }
    for _ in 0..4 {
        assert_no_alloc(|| audio.render_block(&overflow));
        control.flush();
    }
    assert_eq!(audio.rendered_blocks, 604);
    assert_eq!(control.track_capacity, 1024, "the table doubled ten times");
    assert_eq!(audio.tracks.capacity(), 1024);
    assert_eq!(audio.filled(), 600, "every track kept its place");
    assert_eq!(audio.skipped, 0);
    control.collect();
    assert_eq!(control.collected, 10);
}

#[test]
fn a_full_rt_ring_queues_on_the_control_side_and_loses_nothing() {
    let (mut control, mut audio) = sides(2, 8);
    let overflow = LiveOverflow::new();
    for slot in 0..8 {
        control.prepare_track(TrackSlot(slot), 0).unwrap();
    }
    assert_eq!(control.busy, 6, "six messages waited for a slot");
    assert_eq!(control.pending.len(), 6);
    // The audio side keeps rendering; each block frees slots, the control side refills them
    // in order, until everything was delivered.
    for _ in 0..10 {
        assert_no_alloc(|| audio.render_block(&overflow));
        control.flush();
    }
    assert_eq!(audio.applied, 8);
    assert!(control.pending.is_empty());
    assert_eq!(audio.filled(), 8);
}

#[test]
fn a_full_pending_queue_is_engine_busy_for_the_project() {
    // The engine is not consuming (suspended context): the ring fills, then the queue, then
    // the project gets a typed error instead of unbounded parking.
    let (mut control, mut audio) = sides(2, 8);
    control.pending_capacity = 3;
    for slot in 0..5 {
        control.prepare_track(TrackSlot(slot), 0).unwrap();
    }
    assert_eq!(
        control.prepare_track(TrackSlot(5), 0),
        Err(MsgError::EngineBusy)
    );
    assert_eq!(control.pending.len(), 3);
    // Once the engine consumes again, the queue drains in order and sending resumes.
    let overflow = LiveOverflow::new();
    for _ in 0..4 {
        assert_no_alloc(|| audio.render_block(&overflow));
        control.flush();
    }
    assert!(control.pending.is_empty());
    control.prepare_track(TrackSlot(5), 0).unwrap();
    assert_no_alloc(|| audio.render_block(&overflow));
    assert_eq!(audio.filled(), 6);
}

#[test]
fn the_garbage_ring_never_fills_because_prepare_drains_it_first() {
    // Ring of 2: two swaps in flight fill the rt ring; apply pushes two garbage entries,
    // which is exactly the garbage ring's capacity. The third send drains them first, and
    // so does every flush of the queue.
    let (mut control, mut audio) = sides(2, 1);
    let overflow = LiveOverflow::new();
    control.prepare_grow_tracks(2).unwrap();
    control.prepare_grow_tracks(4).unwrap();
    control.prepare_grow_tracks(8).unwrap();
    control.prepare_grow_tracks(16).unwrap();
    assert_eq!(control.pending.len(), 2);
    assert_no_alloc(|| audio.render_block(&overflow));
    assert_eq!(control.collected, 0);
    control.flush();
    assert_eq!(
        control.collected, 2,
        "the flush drained the garbage before pushing"
    );
    assert_no_alloc(|| audio.render_block(&overflow));
    assert_eq!(audio.applied, 4);
    control.collect();
    assert_eq!(control.collected, 4);
}

#[test]
fn removing_a_track_with_its_devices_is_one_garbage_item() {
    let (mut control, mut audio) = sides(2, 1);
    let overflow = LiveOverflow::new();
    control.prepare_track(TrackSlot(0), 8).unwrap();
    assert_no_alloc(|| audio.render_block(&overflow));
    control
        .send(RtMsg::Remove(TableKind::Track(TrackSlot(0))))
        .unwrap();
    // The remove frees the track and its eight devices: one garbage item, no free in apply.
    assert_no_alloc(|| audio.render_block(&overflow));
    assert_eq!(audio.applied, 2);
    assert_eq!(audio.filled(), 0);
    control.collect();
    assert_eq!(control.collected, 1);
}

#[test]
fn a_refused_allocation_is_a_typed_error_not_a_crash() {
    let huge = (isize::MAX as usize) / 2;
    let refused = Table::<Note>::try_with_capacity(huge);
    assert!(
        matches!(refused, Err(MsgError::OutOfMemory { .. })),
        "{refused:?}"
    );
    let (mut control, mut audio) = sides(4, 1);
    let overflow = LiveOverflow::new();
    assert!(matches!(
        control.prepare_grow_tracks(huge),
        Err(MsgError::OutOfMemory { .. })
    ));
    // The audio side never saw anything.
    assert_no_alloc(|| audio.render_block(&overflow));
    assert_eq!(audio.applied, 0);
    assert_eq!(audio.tracks.capacity(), 1);
    // Boxing goes through the same fallible path; a full table gives the item back.
    let boxed = try_box(7u64).unwrap();
    assert_eq!(*boxed, 7);
    let mut full = Table::<u64>::try_with_capacity(0).unwrap();
    assert_eq!(full.push(9), Err(TableFull(9)));
}

#[test]
fn a_limit_is_refused_before_anything_reaches_the_engine() {
    let msg = ControlMsg::ClipNotesUpload {
        clip: CLIP,
        length: Tick(960),
        looped: false,
        first: 0,
        total: limits::MAX_NOTES_PER_CLIP + 1,
        notes: vec![note(0), note(1)],
    };
    assert_eq!(
        msg.check_limits(),
        Err(MsgError::LimitExceeded {
            what: "notes per clip",
            limit: u64::from(limits::MAX_NOTES_PER_CLIP) + 1,
            requested: u64::from(limits::MAX_NOTES_PER_CLIP) + 1,
        })
    );
    // A chunk past its own total is refused too.
    let overrun = ControlMsg::ClipNotesUpload {
        clip: CLIP,
        length: Tick(960),
        looped: false,
        first: 3,
        total: 4,
        notes: vec![note(0), note(1)],
    };
    assert!(matches!(
        overrun.check_limits(),
        Err(MsgError::LimitExceeded {
            what: "notes per clip",
            limit: 4,
            requested: 5
        })
    ));
    let track = ControlMsg::TrackCreate {
        track: TrackSlot(limits::MAX_TRACKS),
        kind: engine::msg::TrackKind::Midi,
    };
    assert!(matches!(
        track.check_limits(),
        Err(MsgError::LimitExceeded { what: "tracks", .. })
    ));
    let last_track = ControlMsg::TrackCreate {
        track: TrackSlot(limits::MAX_TRACKS - 1),
        kind: engine::msg::TrackKind::Midi,
    };
    assert_eq!(last_track.check_limits(), Ok(()));
}

#[test]
fn the_garbage_ring_holds_one_more_than_the_rt_ring_for_the_apply_between_drain_and_push() {
    // The race of a concurrent consumer, replayed by hand: after a drain, C messages are in
    // flight; the audio side applies one (one garbage item) before the next push lands;
    // once everything is applied the garbage ring holds C + 1 items and still had a slot
    // for each of them.
    let (mut control, mut audio) = sides(2, 1);
    let overflow = LiveOverflow::new();
    control.prepare_grow_tracks(2).unwrap();
    control.prepare_grow_tracks(4).unwrap();
    // The audio side applies one message: one garbage item, one rt slot free.
    let held = audio.rt.pop().unwrap();
    assert_no_alloc(|| audio.render_block(&overflow));
    assert_eq!(audio.applied, 1);
    // The next push follows a drain (the garbage ring is emptied) and lands in the free
    // slot; the held message goes back in front of the queue to keep the order.
    control.prepare_grow_tracks(8).unwrap();
    assert_eq!(control.collected, 1);
    control.pending.push_front(held);
    control.flush();
    assert_no_alloc(|| audio.render_block(&overflow));
    // Three messages applied, three garbage items: the second and third went into the ring
    // together with the first still uncollected, which needed the extra slot.
    assert_eq!(audio.applied, 3);
    control.collect();
    assert_eq!(control.collected, 3);
}

#[test]
fn a_full_live_queue_counts_the_overflow_and_the_audio_side_releases_the_notes() {
    let (mut live_tx, mut live_rx) = RingBuffer::<RtMsg>::new(2);
    let overflow = LiveOverflow::new();
    let (_control, mut audio) = sides(2, 1);
    let on = |pitch: u8| RtMsg::NoteOn {
        track: TrackSlot(3),
        pitch,
        velocity: 100,
        offset: 0,
    };
    let mut send = |msg: RtMsg| {
        if live_tx.push(msg).is_err() {
            overflow.record();
        }
    };
    send(on(60));
    send(on(64));
    // The note off of 60 is lost: the player then stops playing, nothing else is sent.
    send(RtMsg::NoteOff {
        track: TrackSlot(3),
        pitch: 60,
        offset: 0,
    });
    assert_eq!(overflow.total(), 1);
    // The next block drains the live queue first (the two note-ons are applied), then sees
    // the counter move and releases every live note, with no further event from the
    // producer: nothing queued before the lost note-off survives the release.
    let mut drained = Vec::new();
    while let Ok(event) = live_rx.pop() {
        drained.push(event);
    }
    assert_eq!(drained.len(), 2);
    assert_no_alloc(|| audio.render_block(&overflow));
    assert_eq!(audio.releases, 1);
    assert_no_alloc(|| audio.render_block(&overflow));
    assert_eq!(
        audio.releases, 1,
        "released once per overflow, not every block"
    );
}

#[test]
fn an_unknown_message_is_skipped_by_its_length_and_the_stream_goes_on() {
    // Two messages: one with a tag this version does not know, then a known one.
    let mut stream = Vec::new();
    let mut unknown = Header {
        version: MSG_VERSION,
        tag: Tag::Stop,
        epoch: 7,
        len: 5,
        seq: 1,
    }
    .encode();
    unknown[2..4].copy_from_slice(&999u16.to_le_bytes());
    stream.extend_from_slice(&unknown);
    stream.extend_from_slice(&[1, 2, 3, 4, 5, 0, 0, 0]); // 5 bytes of payload, padded to 4
    stream.extend_from_slice(
        &Header {
            version: MSG_VERSION,
            tag: Tag::StopAll,
            epoch: 7,
            len: 0,
            seq: 2,
        }
        .encode(),
    );

    let mut at = 0;
    let mut rejected = 0;
    let mut applied = Vec::new();
    while at < stream.len() {
        match Header::decode(&stream[at..], 7) {
            Ok(header) => {
                applied.push(header.tag);
                at += Header::frame_len(header.len);
            }
            Err(MsgError::UnknownTag { tag, skip }) => {
                assert_eq!(tag, 999);
                rejected += 1;
                at += skip;
            }
            Err(other) => panic!("corrupt stream: {other:?}"),
        }
    }
    assert_eq!(rejected, 1);
    assert_eq!(applied, vec![Tag::StopAll]);
    assert_eq!(at, HEADER_BYTES + 8 + HEADER_BYTES);
}

#[test]
fn an_engine_switch_in_the_middle_of_an_upload_drops_the_old_epoch_and_restarts() {
    let header = |tag: Tag, epoch: u16| {
        Header {
            version: MSG_VERSION,
            tag,
            epoch,
            len: 0,
            seq: 0,
        }
        .encode()
    };
    // Chunks of epoch 1 were in flight when the page switched engine and came back.
    let mut current = 1u16;
    assert!(Header::decode(&header(Tag::ClipNotesUpload, 1), current).is_ok());
    // The page resets with epoch 3: adopted because newer (it discards the staging of the
    // cut upload).
    let reset = Header::decode(&header(Tag::Reset, 3), current).unwrap();
    assert_eq!(reset.epoch, 3);
    current = reset.epoch;
    // A chunk of the old upload that arrives late is dropped; the resend of epoch 3 passes.
    assert_eq!(
        Header::decode(&header(Tag::ClipNotesUpload, 1), current),
        Err(MsgError::StaleEpoch {
            message: 1,
            current: 3
        })
    );
    assert!(Header::decode(&header(Tag::ClipNotesUpload, 3), current).is_ok());
    // A Reset of epoch 2 replayed late (a reconnect, a backlog) never rolls the engine
    // back: it is as stale as any other message.
    assert_eq!(
        Header::decode(&header(Tag::Reset, 2), current),
        Err(MsgError::StaleEpoch {
            message: 2,
            current: 3
        })
    );
    // The epoch wraps: 0 is newer than 65535.
    assert_eq!(
        Header::decode(&header(Tag::Reset, 0), 65535).map(|h| h.epoch),
        Ok(0)
    );
    assert!(Header::decode(&header(Tag::Reset, 65535), 0).is_err());
}

/// The chunk staging of the prepare side, as the module doc states it.
struct Staging {
    total: u32,
    notes: Vec<Note>,
    rejected: u32,
}

impl Staging {
    fn chunk(&mut self, first: u32, total: u32, notes: &[Note]) -> Option<Vec<Note>> {
        let fresh = self.notes.is_empty();
        if (fresh && first != 0)
            || (!fresh && (first != self.notes.len() as u32 || total != self.total))
        {
            // Out of order, duplicated or a changed total: restart from this chunk.
            self.rejected += 1;
            self.notes.clear();
            if first != 0 {
                return None;
            }
        }
        if first == 0 {
            self.total = total;
        }
        self.notes.extend_from_slice(notes);
        if self.notes.len() as u32 == self.total {
            Some(core::mem::take(&mut self.notes))
        } else {
            None
        }
    }
}

#[test]
fn a_chunked_upload_reaches_the_engine_whole_or_not_at_all() {
    let mut staging = Staging {
        total: 0,
        notes: Vec::new(),
        rejected: 0,
    };
    let all: Vec<Note> = (0..6).map(note).collect();
    assert_eq!(staging.chunk(0, 6, &all[0..2]), None);
    assert_eq!(staging.chunk(2, 6, &all[2..4]), None);
    // A duplicated chunk restarts the staging: nothing partial is ever visible.
    assert_eq!(staging.chunk(2, 6, &all[2..4]), None);
    assert_eq!(staging.rejected, 1);
    assert_eq!(staging.notes.len(), 0);
    assert_eq!(staging.chunk(0, 6, &all[0..4]), None);
    assert_eq!(staging.chunk(4, 6, &all[4..6]), Some(all.clone()));
    // A changed total restarts as well.
    assert_eq!(staging.chunk(0, 3, &all[0..2]), None);
    assert_eq!(staging.chunk(2, 4, &all[2..4]), None);
    assert_eq!(staging.rejected, 2);
}
