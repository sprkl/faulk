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
//! (`Track::armed`): a note on opens a held note at the playhead of the sample it was played
//! at (below), a note off closes it whether the track is still armed or not
//! (disarming a track during a note does not leave it held), and a note off of a key that
//! is not held (pressed before the capture) is ignored. A second press of a held key closes
//! the first one: a key is held once, so a lost note off never fills the table. The length is measured in samples since
//! the note on, converted at the current tempo, so a note held across a loop wrap keeps the
//! length it was played with. A change of the tempo or of the sample rate during a held note
//! (the project allows both during a take, #513) is followed: when the block's samples per
//! tick change, the samples the held notes have run so far are scaled to the new value
//! ([`Recorder::block`]), so a note keeps the ticks it had run, and runs on from there at
//! the new pace. The change is taken at the start of the block that applies it, so a stamped
//! event (below) older than that block, delivered after the change, is off by its lag times
//! the difference of the two paces (`lag x |1/old - 1/new|` ticks): the stamp's earlier
//! samples run at the new pace. An all-notes-off, a live-queue overflow (the release of every
//! live note) and the end of the capture close the held notes at the start of their block:
//! the notes still held when a take ends are cut there (spec § 11, question 7).
//!
//! # The sample a note was played at (#499)
//!
//! A live note carries the engine sample it was played at when its host gave one
//! (`LiveEvent::NoteOn::at`, the host frame converted by `Engine::begin_block`). By the time
//! it reaches the engine the block that takes it has usually moved past that sample (the
//! note crossed the page, the prepare side and the live queue), so the recorder places it
//! behind the block, at the playhead of that sample ([`Anchors`]: across a loop wrap or a
//! tempo change too). A stamp may also lie ahead of the block: the web page stamps the frame
//! heard at the key press plus the estimated output latency (the take is moved back by the
//! same estimate, `record.commit_take`'s `shift_ms`), and when the estimate is larger than
//! how far the rendering really runs ahead of the output (measured 31 to 42 ms against an
//! estimate of 42 ms in Chromium on Windows, #499), the stamp is a few milliseconds ahead;
//! it is placed there, by the current anchor. The sample it uses ([`Recorder::sample_of`]):
//!
//! - a stamp inside the block, or ahead of it by at most [`MAX_STAMP_DISTANCE_SECONDS`]: the stamp;
//! - a stamp behind the block by at most [`MAX_STAMP_DISTANCE_SECONDS`]: the stamp, but never
//!   before the take began (a note played during the count-in or before the take counts
//!   from the take's first sample, as it did before stamps) nor before the oldest sample the
//!   clock's anchors cover (the last `Play` or `Seek`);
//! - no stamp, or a stamp further than that either way (a stalled page, a clock that means
//!   nothing here): the start of the block, as before stamps existed (128 frames of
//!   uncertainty, 2.7 ms at 48 kHz, plus the time the note took to arrive).
//!
//! A note's length is the samples between the samples of its note on and its note off.
//! Only the recording uses the stamp: the note sounds at the start of the block that takes
//! it, stamped or not (`crate::graph`, `route_live`).
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
use crate::transport::Anchors;

/// How far from its block, behind or ahead, a stamped note may have been played and still be
/// placed at its stamp (module doc): further, it counts from the start of its block.
pub const MAX_STAMP_DISTANCE_SECONDS: f64 = 1.0;

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
    /// The absolute sample of its start (the clock's sample counter), moved when the samples
    /// per tick change so that `sample - from` is always counted at the current pace; signed
    /// because that move can put it before sample 0.
    from: i64,
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
    /// The clock's anchors, read after the block's segments were taken (#499).
    pub anchors: Anchors,
    /// The sample rate, in Hz: how many samples [`MAX_STAMP_DISTANCE_SECONDS`] is.
    pub rate: f64,
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
    /// The first sample of the take in progress: no stamp places a note before it.
    take_from: u64,
    /// Stamped events of the take whose stamp could not be used (module doc).
    unplaced: u32,
    /// The samples per tick of the last block (0 before the first), to see a tempo or rate change.
    last_samples_per_tick: f64,
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
            take_from: 0,
            unplaced: 0,
            last_samples_per_tick: 0.0,
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

    /// Stamped note ons and note offs of the current take whose stamp was not usable (older
    /// than [`MAX_STAMP_DISTANCE_SECONDS`] from their block, either way): they counted from the start of
    /// their block (module doc).
    pub fn unplaced(&self) -> u32 {
        self.unplaced
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
        self.follow_pace(clock);
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
            self.take_from = clock.sample;
            self.unplaced = 0;
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
                    at,
                } => {
                    let at = self.placed(at, clock);
                    // The note's previous press is closed first: a key is held once.
                    self.close(track.0, pitch, at, clock);
                    if velocity > 0 && armed(state, track) {
                        self.open(track.0, pitch, velocity, at, clock);
                    }
                }
                LiveEvent::NoteOff { track, pitch, at } => {
                    let at = self.placed(at, clock);
                    self.close(track.0, pitch, at, clock);
                }
                LiveEvent::AllNotesOff { track } => self.close_all(Some(track.0), clock),
            }
        }
        if release_all {
            self.close_all(None, clock);
        }
    }

    /// A change of the samples per tick since the last block (a tempo change, a new sample rate):
    /// the held notes keep the ticks they have run. Their elapsed samples, counted at the old
    /// pace up to the start of this block, become the same ticks at the new pace.
    fn follow_pace(&mut self, clock: RecordClock) {
        let old = self.last_samples_per_tick;
        let new = clock.samples_per_tick;
        self.last_samples_per_tick = new;
        if !(old > 0.0 && new > 0.0 && new.is_finite() && old != new) {
            return;
        }
        let ratio = new / old;
        let now = i64::try_from(clock.sample).unwrap_or(i64::MAX);
        for held in self.held.as_mut_slice() {
            if held.used {
                let elapsed = now.saturating_sub(held.from).max(0) as f64;
                held.from = now.saturating_sub((elapsed * ratio).round() as i64);
            }
        }
    }

    /// The sample a live event counts from (module doc, "The sample a note was played at"):
    /// its stamp when it is usable, `None` (the start of the block) otherwise.
    pub fn sample_of(&self, at: Option<u64>, clock: RecordClock) -> Option<u64> {
        let at = at?;
        let start = clock.sample;
        let bound = MAX_STAMP_DISTANCE_SECONDS * clock.rate;
        if at >= start {
            return ((at - start) as f64 <= bound).then_some(at);
        }
        if (start - at) as f64 > bound {
            return None;
        }
        Some(at.max(self.take_from).max(clock.anchors.earliest()))
    }

    /// [`Recorder::sample_of`], counting a stamp it cannot use.
    fn placed(&mut self, at: Option<u64>, clock: RecordClock) -> Option<u64> {
        let sample = self.sample_of(at, clock);
        if at.is_some() && sample.is_none() {
            self.unplaced = self.unplaced.saturating_add(1);
        }
        sample
    }

    /// The playhead, in fractional ticks, at the sample an event counts from: the block's
    /// own playhead for one without a usable stamp, as before stamps existed.
    fn position_of(at: Option<u64>, clock: RecordClock) -> f64 {
        at.and_then(|s| clock.anchors.position_of(s))
            .unwrap_or(clock.position)
    }

    fn open(&mut self, track: u32, pitch: u8, velocity: u8, at: Option<u64>, clock: RecordClock) {
        let start = Self::position_of(at, clock).floor();
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
                    from: i64::try_from(at.unwrap_or(clock.sample)).unwrap_or(i64::MAX),
                };
            }
            None => self.dropped = self.dropped.saturating_add(1),
        }
    }

    fn close(&mut self, track: u32, pitch: u8, at: Option<u64>, clock: RecordClock) {
        let mut found = None;
        for (i, h) in self.held.as_slice().iter().enumerate() {
            if h.used && h.track == track && h.pitch == pitch {
                found = Some(i);
                break;
            }
        }
        if let Some(i) = found {
            self.finish(i, at.unwrap_or(clock.sample), clock);
        }
    }

    fn close_all(&mut self, track: Option<u32>, clock: RecordClock) {
        for i in 0..self.held.len() {
            let held = self.held.as_slice().get(i).copied().unwrap_or_default();
            if held.used && track.is_none_or(|t| t == held.track) {
                self.finish(i, clock.sample, clock);
            }
        }
    }

    /// Ends the held note `i` at the absolute sample `at` and puts it in the ring.
    fn finish(&mut self, i: usize, at: u64, clock: RecordClock) {
        let Some(held) = self.held.as_mut_slice().get_mut(i) else {
            return;
        };
        let h = *held;
        held.used = false;
        let samples = i64::try_from(at)
            .unwrap_or(i64::MAX)
            .saturating_sub(h.from)
            .max(0) as f64;
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
