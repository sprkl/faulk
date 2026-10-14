//! Native prototype of the prepare/apply cycle of `engine::msg` (CT-3): a control side that
//! builds tables and an audio side that swaps them under the allocation detector, through
//! every state transition of the cycle, and the attack cases the issue lists. MO-1 turns
//! this into `Engine::prepare` and `Engine::apply`; the contract is what these tests pin.

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use engine::msg::{
    ControlMsg, Garbage, Header, MsgError, Note, Prepared, RtMsg, Swap, Table, TableFull,
    TableKind, TrackSlot, limits,
};
use engine::msg::{ClipSlot, HEADER_BYTES, MSG_VERSION, Tag};
use rtrb::{Consumer, Producer, RingBuffer};

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

const CLIP: ClipSlot = ClipSlot(0);

/// The audio side: the live table and the two rings' ends it holds.
struct AudioSide {
    notes: Table<Note>,
    rt: Consumer<RtMsg>,
    garbage: Producer<Garbage>,
    applied: u32,
    /// Blocks rendered; the "render" here is counting the notes of the table.
    rendered_blocks: u32,
    rendered_notes: u64,
}

impl AudioSide {
    /// `Engine::apply` of MO-1, for the swap: no allocation, no free.
    fn apply_pending(&mut self) {
        for _ in 0..limits::MAX_APPLY_PER_BLOCK {
            let Ok(msg) = self.rt.pop() else { break };
            match msg {
                RtMsg::Swap(Swap {
                    kind: TableKind::ClipNotes(_),
                    next: Prepared::ClipNotes(mut next),
                }) => {
                    next.take_from(&mut self.notes);
                    let old = core::mem::replace(&mut self.notes, next);
                    // The invariant (garbage ring drained before each push) makes this slot free.
                    let pushed = self.garbage.push(Garbage::ClipNotes(old));
                    assert!(pushed.is_ok(), "the garbage ring must never be full in apply");
                }
                _ => unreachable!("this prototype only swaps clip notes"),
            }
            self.applied += 1;
        }
    }

    /// One block: apply what arrived, then "render" by reading the table.
    fn render_block(&mut self) {
        self.apply_pending();
        self.rendered_blocks += 1;
        self.rendered_notes += self.notes.as_slice().len() as u64;
    }
}

/// The control side: prepares the next table and collects the garbage.
struct ControlSide {
    rt: Producer<RtMsg>,
    garbage: Consumer<Garbage>,
    collected: u32,
}

impl ControlSide {
    /// Drains the garbage ring: the drops happen here, never on the audio side.
    fn collect(&mut self) {
        while let Ok(garbage) = self.garbage.pop() {
            drop(garbage);
            self.collected += 1;
        }
    }

    /// `Engine::prepare` of MO-1 for a full table: build the next one, push the swap.
    fn prepare_grow(&mut self, next_capacity: usize) -> Result<(), MsgError> {
        self.collect();
        let next = Table::<Note>::try_with_capacity(next_capacity)?;
        self.rt
            .push(RtMsg::Swap(Swap {
                kind: TableKind::ClipNotes(CLIP),
                next: Prepared::ClipNotes(next),
            }))
            .map_err(|_| MsgError::EngineBusy)
    }
}

fn sides(ring_capacity: usize, table_capacity: usize) -> (ControlSide, AudioSide) {
    let (rt_tx, rt_rx) = RingBuffer::new(ring_capacity);
    let (gc_tx, gc_rx) = RingBuffer::new(ring_capacity);
    (
        ControlSide {
            rt: rt_tx,
            garbage: gc_rx,
            collected: 0,
        },
        AudioSide {
            notes: Table::try_with_capacity(table_capacity).unwrap(),
            rt: rt_rx,
            garbage: gc_tx,
            applied: 0,
            rendered_blocks: 0,
            rendered_notes: 0,
        },
    )
}

fn note(i: usize) -> Note {
    Note {
        start: i as i64 * 240,
        length: 240,
        pitch: (i % 128) as u8,
        velocity: 100,
    }
}

#[test]
fn the_cycle_goes_through_every_transition_without_allocating_on_the_audio_side() {
    let (mut control, mut audio) = sides(limits::RT_RING_CAPACITY, 2);

    // empty → filling → full: the push refuses typed, the table is unchanged.
    assert!(audio.notes.is_empty());
    assert_no_alloc(|| {
        audio.notes.push(note(0)).unwrap();
        audio.notes.push(note(1)).unwrap();
        assert!(audio.notes.is_full());
        assert_eq!(audio.notes.push(note(2)), Err(TableFull));
    });
    assert_eq!(audio.notes.len(), 2);

    // full → growing: prepare builds the next table (this allocates, off the audio side).
    let next_capacity = audio.notes.next_capacity();
    assert_eq!(next_capacity, 4);
    control.prepare_grow(next_capacity).unwrap();
    assert_eq!(audio.notes.capacity(), 2, "nothing changes before apply");

    // growing → swapped: apply moves the items and swaps, with no allocation and no free.
    assert_no_alloc(|| audio.render_block());
    assert_eq!(audio.applied, 1);
    assert_eq!(audio.notes.capacity(), 4);
    assert_eq!(audio.notes.as_slice(), &[note(0), note(1)]);
    assert_no_alloc(|| {
        audio.notes.push(note(2)).unwrap();
        audio.notes.push(note(3)).unwrap();
    });

    // swapped → collected: the old table is freed on the control side.
    assert_eq!(control.collected, 0);
    control.collect();
    assert_eq!(control.collected, 1);
}

#[test]
fn growth_during_playback_loses_nothing_and_skips_no_block() {
    let (mut control, mut audio) = sides(16, 1);
    let mut next_note = 0usize;
    let mut grows = 0;
    for _block in 0..2000 {
        // The project keeps adding notes while the audio side renders every block.
        if audio.notes.push(note(next_note)) == Err(TableFull) {
            control.prepare_grow(audio.notes.next_capacity()).unwrap();
            grows += 1;
        } else {
            next_note += 1;
        }
        assert_no_alloc(|| audio.render_block());
    }
    assert_eq!(audio.rendered_blocks, 2000);
    assert_eq!(grows, audio.applied);
    assert!(grows >= 10, "the table doubled at least ten times: {grows}");
    let notes = audio.notes.as_slice();
    assert_eq!(notes.len(), next_note);
    for (i, n) in notes.iter().enumerate() {
        assert_eq!(*n, note(i), "note {i} kept its place through the swaps");
    }
    control.collect();
    assert_eq!(control.collected, grows);
}

#[test]
fn a_full_rt_ring_makes_prepare_busy_and_blocks_nobody() {
    let (mut control, mut audio) = sides(2, 1);
    control.prepare_grow(2).unwrap();
    control.prepare_grow(4).unwrap();
    assert_eq!(control.prepare_grow(8), Err(MsgError::EngineBusy));
    // The audio side keeps rendering; once it applied, prepare goes through again.
    assert_no_alloc(|| audio.render_block());
    assert_eq!(audio.applied, 2);
    control.prepare_grow(8).unwrap();
}

#[test]
fn the_garbage_ring_never_fills_because_prepare_drains_it_first() {
    // Ring of 2: two swaps in flight fill the rt ring; apply pushes two garbage entries,
    // which is exactly the garbage ring's capacity. A third prepare drains them first.
    let (mut control, mut audio) = sides(2, 1);
    control.prepare_grow(2).unwrap();
    control.prepare_grow(4).unwrap();
    assert_no_alloc(|| audio.render_block());
    assert_eq!(control.collected, 0);
    control.prepare_grow(8).unwrap();
    assert_eq!(control.collected, 2);
    assert_no_alloc(|| audio.render_block());
    assert_eq!(audio.applied, 3);
}

#[test]
fn a_refused_allocation_is_a_typed_error_not_a_crash() {
    let huge = (isize::MAX as usize) / 2;
    let refused = Table::<Note>::try_with_capacity(huge);
    assert!(matches!(refused, Err(MsgError::OutOfMemory { .. })), "{refused:?}");
    let (mut control, mut audio) = sides(4, 1);
    assert!(matches!(control.prepare_grow(huge), Err(MsgError::OutOfMemory { .. })));
    // The audio side never saw anything.
    assert_no_alloc(|| audio.render_block());
    assert_eq!(audio.applied, 0);
    assert_eq!(audio.notes.capacity(), 1);
}

#[test]
fn a_limit_is_refused_before_anything_reaches_the_engine() {
    let msg = ControlMsg::ClipNotesUpload {
        clip: CLIP,
        length: 960,
        looped: false,
        first: limits::MAX_NOTES_PER_CLIP - 1,
        total: 2,
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

/// The live input queue: bounded, overflow counted, no stuck note.
struct LiveSender {
    queue: Producer<RtMsg>,
    overflows: u32,
    /// Set by an overflow: the next push is `AllNotesOff` of the track that lost an event.
    pending_all_off: Option<TrackSlot>,
}

impl LiveSender {
    fn send(&mut self, msg: RtMsg) {
        if let Some(track) = self.pending_all_off.take() {
            if self.queue.push(RtMsg::AllNotesOff { track }).is_err() {
                self.pending_all_off = Some(track);
                self.overflows += 1;
                return;
            }
        }
        if let Err(rtrb::PushError::Full(lost)) = self.queue.push(msg) {
            self.overflows += 1;
            let track = match lost {
                RtMsg::NoteOn { track, .. }
                | RtMsg::NoteOff { track, .. }
                | RtMsg::AllNotesOff { track } => track,
                _ => unreachable!("only live events go through this queue"),
            };
            self.pending_all_off = Some(track);
        }
    }
}

#[test]
fn a_full_live_queue_counts_the_overflow_and_releases_the_notes_first() {
    let (tx, mut rx) = RingBuffer::new(2);
    let mut live = LiveSender {
        queue: tx,
        overflows: 0,
        pending_all_off: None,
    };
    let on = |pitch: u8| RtMsg::NoteOn {
        track: TrackSlot(3),
        pitch,
        velocity: 100,
        offset: 0,
    };
    live.send(on(60));
    live.send(on(64));
    live.send(on(67));
    assert_eq!(live.overflows, 1);
    assert_eq!(live.pending_all_off, Some(TrackSlot(3)));
    // The audio side drains one slot; the next send puts AllNotesOff before anything else.
    assert!(matches!(rx.pop(), Ok(RtMsg::NoteOn { pitch: 60, .. })));
    live.send(on(72));
    assert!(matches!(rx.pop(), Ok(RtMsg::NoteOn { pitch: 64, .. })));
    assert!(matches!(
        rx.pop(),
        Ok(RtMsg::AllNotesOff {
            track: TrackSlot(3)
        })
    ));
    // The note that followed the release was lost too (the ring had one free slot), counted.
    assert_eq!(live.overflows, 2);
    assert_eq!(live.pending_all_off, Some(TrackSlot(3)));
}

#[test]
fn an_unknown_message_is_skipped_by_its_length_and_the_stream_goes_on() {
    // Two messages: one with a tag this version does not know, then a known one.
    let mut stream = Vec::new();
    let unknown = Header {
        version: MSG_VERSION,
        tag: Tag::Reset,
        epoch: 7,
        len: 5,
        seq: 1,
    }
    .encode();
    let mut unknown = unknown;
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
fn a_message_of_the_previous_engine_is_dropped_after_a_switch() {
    let in_flight = Header {
        version: MSG_VERSION,
        tag: Tag::Play,
        epoch: 1,
        len: 8,
        seq: 41,
    }
    .encode();
    // The page switched engine: epoch 2. The message of epoch 1 that was in flight is dropped.
    assert_eq!(
        Header::decode(&in_flight, 2),
        Err(MsgError::StaleEpoch {
            message: 1,
            current: 2
        })
    );
    assert!(Header::decode(&in_flight, 1).is_ok());
}
