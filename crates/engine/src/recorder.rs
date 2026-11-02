//! MIDI recording in the engine (RE-2, `docs/specs/03-enregistrement-midi.md` § 7 and § 8): the
//! notes played live on the armed tracks while the transport records, stamped in transport
//! ticks by the engine, never by the page.
//!
//! # When it captures
//!
//! Capture is on while the clock runs, the playback asks for recording (`SetRecording`) and
//! the count-in is over: a recording started from a stopped clock with `count_in_bars > 0`
//! runs the playhead from `count_in_bars` before the cue ([`crate::engine`], `start_clock`),
//! and the recorder waits for the block that reaches the cue (its notes are stamped at the
//! start of that block, just before the cue). A note played during the count-in is heard,
//! not captured, and not kept for later. Each time capture switches on a
//! new **take** begins: its number goes up by one and its notes count from 0.
//!
//! # What it captures
//!
//! The live events of the block ([`LiveEvent`], the live queue) on an **armed** MIDI track
//! (`Track::armed`): a note on opens a held note at the playhead of the start of the block
//! (the events of a block are applied at its start, so a note is stamped to the block: 128
//! frames, 2.7 ms at 48 kHz), a note off closes it whether the track is still armed or not
//! (disarming a track during a note does not leave it held), and a note off of a key that
//! is not held (pressed before the capture) is ignored. A second press of a held key closes
//! the first one: a key is held once, so a lost note off never fills the table. The length is measured in samples since
//! the note on, converted at the current tempo, so a note held across a loop wrap keeps the
//! length it was played with (the tempo is constant in the MVP: a tempo change during a
//! held note converts the whole length at the new tempo). An all-notes-off, a live-queue overflow (the release of every
//! live note) and the end of the capture close the held notes at the start of their block:
//! the notes still held when a take ends are cut there (spec § 11, question 7).
//!
//! # Where the notes go
//!
//! A finished note goes into a ring of [`RING`] notes, indexed by its number in the take.
//! The report ([`crate::host`]) carries the take's number, its count and the last
//! `min(count, RING)` notes, so a reader that misses reports loses nothing until it falls
//! [`RING`] notes behind, which it can tell from the indices. A note that finds the held
//! table full (more than [`MAX_HELD`] keys held at once on the armed tracks) is not recorded
//! and is counted in [`Recorder::dropped`], never hidden.
//!
//! Nothing here allocates, frees, locks or panics: the tables are built with the engine.

use crate::engine::State;
use crate::msg::{
    LiveEvent, MAX_RECORDED_PER_REPORT, MsgError, Note, RecordedNote, Table, TrackKind, TrackSlot,
};
use crate::time::Tick;

/// Keys held at once on the armed tracks that the recorder follows.
pub const MAX_HELD: usize = 256;

/// Finished notes kept for the report: as many as one report carries.
pub const RING: usize = MAX_RECORDED_PER_REPORT;

/// A held note: where and when it started.
#[derive(Clone, Copy, Debug, Default)]
struct Held {
    used: bool,
    track: u32,
    pitch: u8,
    velocity: u8,
    /// The transport tick of its start.
    start: i64,
    /// The absolute sample of its start (the clock's sample counter).
    from: u64,
}

impl Default for RecordedNote {
    fn default() -> Self {
        RecordedNote {
            track: TrackSlot(0),
            note: Note::default(),
        }
    }
}

/// The clock of the block the recorder reads (the start of the block in progress).
#[derive(Clone, Copy, Debug)]
pub struct RecordClock {
    /// The clock runs.
    pub running: bool,
    /// The playback asks for recording.
    pub recording: bool,
    /// The playhead in fractional ticks.
    pub position: f64,
    /// The absolute sample of the start of the block.
    pub sample: u64,
    /// Samples per tick at the current tempo.
    pub samples_per_tick: f64,
    /// Frames of the block.
    pub frames: usize,
}

impl RecordClock {
    /// The playhead at the end of the block, in fractional ticks.
    fn end(&self) -> f64 {
        if self.samples_per_tick > 0.0 {
            self.position + self.frames as f64 / self.samples_per_tick
        } else {
            self.position
        }
    }
}

/// The engine's recorder. See the module documentation.
#[derive(Debug)]
pub struct Recorder {
    held: Table<Held>,
    ring: Table<RecordedNote>,
    take: u32,
    total: u32,
    dropped: u32,
    capturing: bool,
    /// The cue a count-in leads to: capture waits until the playhead reaches it.
    count_in_to: Option<i64>,
}

impl Recorder {
    /// The recorder's tables, built off the audio path.
    pub fn try_new() -> Result<Self, MsgError> {
        Ok(Recorder {
            held: Table::try_filled(MAX_HELD)?,
            ring: Table::try_filled(RING)?,
            take: 0,
            total: 0,
            dropped: 0,
            capturing: false,
            count_in_to: None,
        })
    }

    /// A count-in starts and leads to `cue`: capture waits until the playhead reaches it.
    pub fn count_in(&mut self, cue: Tick) {
        self.count_in_to = Some(cue.0);
    }

    /// The tick a running count-in leads to, if one runs.
    pub fn counting_in(&self) -> Option<Tick> {
        self.count_in_to.map(Tick)
    }

    /// Whether the recorder captures (after the last [`Recorder::block`]).
    pub fn capturing(&self) -> bool {
        self.capturing
    }

    /// The number of the current (or last) take: 0 before the first.
    pub fn take(&self) -> u32 {
        self.take
    }

    /// Notes finished in the current take.
    pub fn total(&self) -> u32 {
        self.total
    }

    /// Notes of the current take lost because the held table was full.
    pub fn dropped(&self) -> u32 {
        self.dropped
    }

    /// The last `min(total, RING)` notes of the take, in their order, with their index.
    pub fn recent(&self, mut each: impl FnMut(u32, &RecordedNote)) {
        let total = self.total as usize;
        let n = total.min(RING);
        let ring = self.ring.as_slice();
        for index in total - n..total {
            if let Some(note) = ring.get(index % RING) {
                each(index as u32, note);
            }
        }
    }

    /// One block: switches capture on or off from the clock, then takes the block's live
    /// events (`release_all`: the live queue overflowed, every live note is released after
    /// them). No allocation, no panic.
    pub fn block(
        &mut self,
        state: &State,
        clock: RecordClock,
        live: &[LiveEvent],
        release_all: bool,
    ) {
        let wanted = clock.running && clock.recording;
        if let Some(to) = self.count_in_to
            && (!wanted || clock.end() > to as f64)
        {
            self.count_in_to = None;
        }
        let on = wanted && self.count_in_to.is_none();
        if on && !self.capturing {
            self.take = self.take.wrapping_add(1);
            self.total = 0;
            self.dropped = 0;
            for held in self.held.as_mut_slice() {
                held.used = false;
            }
            self.capturing = true;
        } else if !on && self.capturing {
            self.close_all(None, clock);
            self.capturing = false;
        }
        if !self.capturing {
            return;
        }
        for event in live {
            match *event {
                LiveEvent::NoteOn {
                    track,
                    pitch,
                    velocity,
                } => {
                    // The note's previous press is closed first: a key is held once.
                    self.close(track.0, pitch, clock);
                    if velocity > 0 && armed(state, track) {
                        self.open(track.0, pitch, velocity, clock);
                    }
                }
                LiveEvent::NoteOff { track, pitch } => self.close(track.0, pitch, clock),
                LiveEvent::AllNotesOff { track } => self.close_all(Some(track.0), clock),
            }
        }
        if release_all {
            self.close_all(None, clock);
        }
    }

    fn open(&mut self, track: u32, pitch: u8, velocity: u8, clock: RecordClock) {
        let start = clock.position.floor();
        let start = if start.is_finite() {
            start.clamp(i64::MIN as f64, i64::MAX as f64) as i64
        } else {
            0
        };
        match self.held.as_mut_slice().iter_mut().find(|h| !h.used) {
            Some(slot) => {
                *slot = Held {
                    used: true,
                    track,
                    pitch,
                    velocity,
                    start,
                    from: clock.sample,
                };
            }
            None => self.dropped = self.dropped.saturating_add(1),
        }
    }

    fn close(&mut self, track: u32, pitch: u8, clock: RecordClock) {
        let mut found = None;
        for (i, h) in self.held.as_slice().iter().enumerate() {
            if h.used && h.track == track && h.pitch == pitch {
                found = Some(i);
                break;
            }
        }
        if let Some(i) = found {
            self.finish(i, clock);
        }
    }

    fn close_all(&mut self, track: Option<u32>, clock: RecordClock) {
        for i in 0..self.held.len() {
            let held = self.held.as_slice().get(i).copied().unwrap_or_default();
            if held.used && track.is_none_or(|t| t == held.track) {
                self.finish(i, clock);
            }
        }
    }

    /// Ends the held note `i` at the start of this block and puts it in the ring.
    fn finish(&mut self, i: usize, clock: RecordClock) {
        let Some(held) = self.held.as_mut_slice().get_mut(i) else {
            return;
        };
        let h = *held;
        held.used = false;
        let samples = clock.sample.saturating_sub(h.from) as f64;
        let ticks = if clock.samples_per_tick > 0.0 {
            (samples / clock.samples_per_tick).round()
        } else {
            0.0
        };
        let length = if ticks.is_finite() {
            ticks.clamp(1.0, i64::MAX as f64) as i64
        } else {
            1
        };
        let note = RecordedNote {
            track: TrackSlot(h.track),
            note: Note {
                start: Tick(h.start),
                length: Tick(length),
                pitch: h.pitch,
                velocity: h.velocity,
                ..Note::default()
            },
        };
        let at = self.total as usize % RING;
        if let Some(slot) = self.ring.as_mut_slice().get_mut(at) {
            *slot = note;
        }
        self.total = self.total.wrapping_add(1);
    }
}

/// Whether `track` is an armed MIDI track of the engine's state.
fn armed(state: &State, track: TrackSlot) -> bool {
    state
        .tracks
        .as_slice()
        .get(track.0 as usize)
        .and_then(Option::as_ref)
        .is_some_and(|t| t.armed && t.kind == TrackKind::Midi)
}
