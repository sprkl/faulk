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
//! start of that block, just before the cue). Each time capture switches on a new **take**
//! begins: its number goes up by one and its notes count from 0.
//!
//! # The count-in (#723)
//!
//! A note played during the count-in is heard. Its event usually reaches the engine before the
//! cue's block when the musician anticipates the first beat by more than the rendering runs
//! ahead of the output (31 to 42 ms in the web build, #499), so the recorder reads the
//! count-in's live events too. A note on on an armed MIDI track whose heard position (its
//! sample, below, moved back by the shift of the take to come, placed by the clock's anchors:
//! the same rules as during the take) falls within [`COUNT_IN_EARLY`] (a sixteenth, the
//! tolerance of `record.commit_take`'s early rule) before the cue is held for that take, at its
//! heard position, before the cue, exactly as a note heard as early whose event arrives once
//! the capture has begun; one heard earlier is counted lost ([`Recorder::dropped`] of the take
//! to come), never hidden. A note off, a second press of the key, an all-notes-off or an
//! overflow ends a held one: it waits, with its played length, and becomes one of the take's
//! first notes when the take begins; one still held then is a held note of the take, ended by
//! its note off as any other. The shift of the take to come is taken at every block of the
//! count-in: a note is placed (and kept or not) with the shift in force when its event arrives,
//! and one held while a `SetRecording` changes the shift before the cue keeps the length it was
//! played with (its start stays where it was placed).
//! A count-in cancelled before its cue (a stop, the recording turned off) forgets what it kept
//! and what it counted. A stamp the count-in cannot use counts in the take's
//! [`Recorder::unplaced`] and its event counts from the start of its block, as in the take.
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
//! heard at the key press plus the estimated output latency (the recorder moves the note back
//! by the same estimate, the take's shift below), and when the estimate is larger than
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
//! # The take's shift (#560)
//!
//! `SetRecording` carries how much earlier the take's notes are placed (`shift_us`: the
//! output latency the host estimates plus the manual offset; negative: later). The recorder
//! takes it when a take begins (a later `SetRecording` changes the next take only) and
//! applies it on its own clock, in whole samples at the current rate: every sample a note
//! starts or ends at (the sample above, or the start of its block) is moved back by the
//! shift, then placed at the playhead of that sample by the clock's anchors. The anchors know
//! where the loop wrapped, so the shift follows the playhead as it was heard:
//!
//! - a note heard just before the loop's end whose stamp fell after the wrap goes back before
//!   the loop's end, where it was heard;
//! - a note heard just before the loop's start while the playhead enters the loop from before
//!   it (the first pass, a count-in that ends on the loop's start included) stays before the
//!   loop's start;
//! - the stamp, before the shift, is never before the take's first sample (the rule above), so
//!   a note heard less than one shift before the take began is kept where it was heard, before
//!   the take's first tick, and one heard earlier lands one shift before that tick; the
//!   recorder drops none of those it takes (`record.commit_take` drops a note before tick 0 or
//!   before its clip). An event that reaches the engine during the count-in is placed the same
//!   way, and kept when it falls within [`COUNT_IN_EARLY`] before the cue ("The count-in"
//!   above). A moved sample older than the oldest anchor (a note heard just before the
//!   take's `Play`, or before a seek or a sample-rate change) is placed on the line, back from
//!   that anchor. The anchors hold the anchor in force and the one before it: a sample older
//!   than both after two loop wraps or a wrap and a tempo change within the shift and the
//!   note's lag (a loop shorter than about the latency) is placed back from the oldest one, on
//!   the line, not round the loop;
//! - a negative shift moves a sample ahead of the block, which the anchor in force places on the
//!   line: a position it gives past the end of the loop the playhead will wrap at (inside
//!   the loop, or before it on the first pass) is folded back into the loop, where the playhead
//!   will be.
//!
//! A note's on and off move by the same shift, so its length does not change. A take without
//! a shift places its notes exactly as before the shift existed, except a stamp ahead of the
//! block past the loop's end, now folded into the loop too.
//!
//! # Where the notes go
//!
//! A finished note goes into a ring of [`RING`] notes, indexed by its number in the take.
//! The report ([`crate::host`]) carries the take's number, its count and the last
//! `min(count, RING)` notes, so a reader that misses reports loses nothing until it falls
//! [`RING`] notes behind, which it can tell from the indices. A note that finds the held
//! table full (more than [`MAX_HELD`] keys held at once on the armed tracks) is not recorded
//! and is counted in [`Recorder::dropped`], never hidden, as is a note of the count-in heard
//! too early to be kept ("The count-in" above) or ended with [`MAX_HELD`] released notes of the
//! count-in already waiting.
//!
//! Nothing here allocates, frees, locks or panics: the tables are built with the engine.

use crate::engine::State;
use crate::msg::{
    LiveEvent, MAX_RECORDED_PER_REPORT, MsgError, Note, RecordedNote, Table, TrackKind, TrackSlot,
    limits,
};
use crate::time::{LoopRegion, PPQ, Tick};
use crate::transport::Anchors;

/// How far from its block, behind or ahead, a stamped note may have been played and still be
/// placed at its stamp (module doc): further, it counts from the start of its block.
pub const MAX_STAMP_DISTANCE_SECONDS: f64 = 1.0;

/// How early before the cue a note heard during a count-in may be and still be a note of the
/// take, in ticks: a sixteenth, the tolerance of `record.commit_take`'s early rule (#723,
/// module doc, "The count-in").
pub const COUNT_IN_EARLY: i64 = PPQ / 4;

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
    /// How much earlier the notes of a take that starts are placed, in microseconds
    /// (`ControlMsg::SetRecording`, #560; module doc, "The take's shift").
    pub shift_us: i32,
    /// The loop the playhead will wrap at the end of (`Transport::armed_loop`): armed inside
    /// the loop and before its start on the first pass.
    pub looping: Option<LoopRegion>,
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
    /// The take's shift, taken when the take began (module doc), in microseconds.
    shift_us: i32,
    /// [`Recorder::shift_us`] in whole samples at [`Recorder::shift_rate`].
    shift: i64,
    /// The sample rate `shift` was counted at.
    shift_rate: f64,
    /// Notes kept from the count-in in progress and already released (#723, module doc): the
    /// take's first notes when it begins.
    early: Table<RecordedNote>,
    /// How many of [`Recorder::early`] are used.
    early_len: usize,
    /// The count-in's lost notes: the take's [`Recorder::dropped`] when it begins.
    early_dropped: u32,
    /// The count-in's unusable stamps: the take's [`Recorder::unplaced`] when it begins.
    early_unplaced: u32,
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
            shift_us: 0,
            shift: 0,
            shift_rate: 0.0,
            early: Table::try_filled(MAX_HELD)?,
            early_len: 0,
            early_dropped: 0,
            early_unplaced: 0,
        })
    }

    /// A count-in starts and leads to `cue`: capture waits until the playhead reaches it. What
    /// an earlier count-in kept is forgotten; while a take still captures (a stop and a new
    /// recording applied before the next block), its held notes stay, for the next block to
    /// close in that take.
    pub fn count_in(&mut self, cue: Tick) {
        self.count_in_to = Some(cue.0);
        self.forget_early(!self.capturing);
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

    /// Notes of the current take lost: the held table was full, or they were played during its
    /// count-in, too early before the cue to be kept (module doc, "The count-in").
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
        let counted_in = self.count_in_to.is_some();
        if let Some(to) = self.count_in_to
            && (!wanted || clock.end() > to as f64)
        {
            self.count_in_to = None;
        }
        let on = wanted && self.count_in_to.is_none();
        if on && !self.capturing {
            // Without a count-in just ended, nothing kept from one belongs to this take.
            if !counted_in {
                self.forget_early(true);
            }
            self.take = self.take.wrapping_add(1);
            self.total = 0;
            self.dropped = self.early_dropped;
            self.unplaced = self.early_unplaced;
            self.capturing = true;
            self.take_from = clock.sample;
            self.adopt_shift(clock);
            // The count-in's released notes are the take's first; its held ones stay held.
            for i in 0..self.early_len {
                if let Some(note) = self.early.as_slice().get(i).copied() {
                    self.push(note);
                }
            }
            self.forget_early(false);
        } else if !on && self.capturing {
            // Closed on the ended take's shift, before a count-in that starts here takes its own.
            self.close_all(None, clock);
            self.capturing = false;
        }
        if counted_in && !on && self.count_in_to.is_none() {
            // The count-in was cancelled before its cue (a stop, the recording turned off).
            self.forget_early(true);
        }
        // During a count-in its events are read too (module doc, "The count-in"): a note on is
        // held only when heard within `COUNT_IN_EARLY` before the cue, and what it counts
        // and ends waits for the take to come.
        let window = match self.count_in_to {
            Some(cue) => {
                self.adopt_shift(clock);
                Some(cue.saturating_sub(COUNT_IN_EARLY) as f64)
            }
            None if self.capturing => None,
            None => return,
        };
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
                        let kept = window.is_none_or(|from| self.start_of(at, clock) >= from)
                            && self.open(track.0, pitch, velocity, at, clock);
                        if !kept {
                            self.lose();
                        }
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

    /// Counts a lost note: in the take, or, during a count-in, for the take to come.
    fn lose(&mut self) {
        let lost = if self.capturing {
            &mut self.dropped
        } else {
            &mut self.early_dropped
        };
        *lost = lost.saturating_add(1);
    }

    /// Forgets what a count-in kept: its released notes and its counts, and its held notes too
    /// when `held` (the held table then holds only the count-in's).
    fn forget_early(&mut self, held: bool) {
        self.early_len = 0;
        self.early_dropped = 0;
        self.early_unplaced = 0;
        if held {
            for h in self.held.as_mut_slice() {
                h.used = false;
            }
        }
    }

    /// Takes the shift of the take to come (`clock.shift_us`, clamped) at the clock's rate: the
    /// held notes' starts move by the difference, so that they stay on its clock.
    fn adopt_shift(&mut self, clock: RecordClock) {
        let us = clock
            .shift_us
            .clamp(-limits::MAX_RECORD_SHIFT_US, limits::MAX_RECORD_SHIFT_US);
        self.move_shift(us, clock.rate);
    }

    /// Sets the shift to `us` at `rate`, moving the held notes' starts by the difference.
    fn move_shift(&mut self, us: i32, rate: f64) {
        let shift = shift_samples(us, rate);
        let delta = self.shift.saturating_sub(shift);
        if delta != 0 {
            for held in self.held.as_mut_slice() {
                if held.used {
                    held.from = held.from.saturating_add(delta);
                }
            }
        }
        self.shift_us = us;
        self.shift = shift;
        self.shift_rate = rate;
    }

    /// A change of the samples per tick since the last block (a tempo change, a new sample rate):
    /// the held notes keep the ticks they have run. Their elapsed samples, counted at the old
    /// pace up to the start of this block, become the same ticks at the new pace. A new sample
    /// rate also gives the take's shift a new count of samples: the held notes' starts move by
    /// the difference first, so that they stay on the take's clock.
    fn follow_pace(&mut self, clock: RecordClock) {
        if self.capturing && clock.rate != self.shift_rate {
            self.move_shift(self.shift_us, clock.rate);
        }
        let old = self.last_samples_per_tick;
        let new = clock.samples_per_tick;
        self.last_samples_per_tick = new;
        if !(old > 0.0 && new > 0.0 && new.is_finite() && old != new) {
            return;
        }
        let ratio = new / old;
        let now = self.heard(clock.sample);
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
        usable(at, clock, self.take_from)
    }

    /// [`Recorder::sample_of`], counting a stamp it cannot use. During a count-in no take has
    /// begun: no take's first sample bounds the stamp, and an unusable one counts for the take
    /// to come.
    fn placed(&mut self, at: Option<u64>, clock: RecordClock) -> Option<u64> {
        let (floor, unplaced) = if self.capturing {
            (self.take_from, &mut self.unplaced)
        } else {
            (0, &mut self.early_unplaced)
        };
        let sample = usable(at, clock, floor);
        if at.is_some() && sample.is_none() {
            *unplaced = unplaced.saturating_add(1);
        }
        sample
    }

    /// The absolute sample `sample` on the take's clock: moved back by the take's shift
    /// (forward for a negative one), saturating; signed, as a held note's `from`.
    fn heard(&self, sample: u64) -> i64 {
        i64::try_from(sample)
            .unwrap_or(i64::MAX)
            .saturating_sub(self.shift)
    }

    /// The sample on the take's clock of an event that counts from `at` ([`Recorder::placed`]),
    /// or from the start of its block.
    fn event_sample(&self, at: Option<u64>, clock: RecordClock) -> i64 {
        self.heard(at.unwrap_or(clock.sample))
    }

    /// The playhead, in fractional ticks, where an event that counts from `at` was heard
    /// (module doc, "The take's shift"): the block's own playhead for one without a usable
    /// stamp in a take without a shift, as before stamps existed.
    fn start_of(&self, at: Option<u64>, clock: RecordClock) -> f64 {
        if at.is_none() && self.shift == 0 {
            return clock.position;
        }
        let sample = self.event_sample(at, clock);
        let anchors = clock.anchors;
        let earliest = anchors.earliest();
        let position = match u64::try_from(sample) {
            Ok(s) if s >= earliest => anchors.position_of(s),
            // Before the oldest anchor (a note heard less than one shift before the take or a
            // `Play` began): on the line, back from it.
            _ if clock.samples_per_tick > 0.0 => anchors.position_of(earliest).map(|p| {
                p - (i64::try_from(earliest)
                    .unwrap_or(i64::MAX)
                    .saturating_sub(sample)) as f64
                    / clock.samples_per_tick
            }),
            _ => None,
        }
        .unwrap_or(clock.position);
        // Past the end of the loop the playhead is inside of: a sample ahead of the block (a
        // negative shift), which the current anchor does not know will wrap.
        match clock.looping {
            Some(region) => {
                let (from, to) = (region.start().0 as f64, region.end().0 as f64);
                if to > from && position >= to {
                    from + (position - to).rem_euclid(to - from)
                } else {
                    position
                }
            }
            None => position,
        }
    }

    /// Holds a new note heard at `at` (or the start of its block); `false` when the held table
    /// is full: the caller counts it lost.
    fn open(
        &mut self,
        track: u32,
        pitch: u8,
        velocity: u8,
        at: Option<u64>,
        clock: RecordClock,
    ) -> bool {
        let start = self.start_of(at, clock).floor();
        let start = if start.is_finite() {
            start.clamp(i64::MIN as f64, i64::MAX as f64) as i64
        } else {
            0
        };
        let from = self.event_sample(at, clock);
        match self.held.as_mut_slice().iter_mut().find(|h| !h.used) {
            Some(slot) => {
                *slot = Held {
                    used: true,
                    track,
                    pitch,
                    velocity,
                    start,
                    from,
                };
                true
            }
            None => false,
        }
    }

    /// The held note of `track` and `pitch`, if one is held.
    fn held_index(&self, track: u32, pitch: u8) -> Option<usize> {
        self.held
            .as_slice()
            .iter()
            .position(|h| h.used && h.track == track && h.pitch == pitch)
    }

    fn close(&mut self, track: u32, pitch: u8, at: Option<u64>, clock: RecordClock) {
        if let Some(i) = self.held_index(track, pitch) {
            self.finish(i, self.event_sample(at, clock), clock);
        }
    }

    fn close_all(&mut self, track: Option<u32>, clock: RecordClock) {
        for i in 0..self.held.len() {
            let held = self.held.as_slice().get(i).copied().unwrap_or_default();
            if held.used && track.is_none_or(|t| t == held.track) {
                self.finish(i, self.heard(clock.sample), clock);
            }
        }
    }

    /// Ends the held note `i` at the absolute sample `at` (on the take's clock, the shift
    /// applied) and puts it in the ring; during a count-in, in [`Recorder::early`] for the take
    /// to come (counted lost when it is full).
    fn finish(&mut self, i: usize, at: i64, clock: RecordClock) {
        let Some(note) = self.take_note(i, at, clock) else {
            return;
        };
        if self.capturing {
            self.push(note);
            return;
        }
        match self.early.as_mut_slice().get_mut(self.early_len) {
            Some(slot) => {
                *slot = note;
                self.early_len += 1;
            }
            None => self.lose(),
        }
    }

    /// Frees the held note `i` and gives it ended at the absolute sample `at` (on the take's
    /// clock): its length counted at the block's pace, one tick at least.
    fn take_note(&mut self, i: usize, at: i64, clock: RecordClock) -> Option<RecordedNote> {
        let held = self.held.as_mut_slice().get_mut(i)?;
        let h = *held;
        if !h.used {
            return None;
        }
        held.used = false;
        let samples = at.saturating_sub(h.from).max(0) as f64;
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
        Some(RecordedNote {
            track: TrackSlot(h.track),
            note: Note {
                start: Tick(h.start),
                length: Tick(length),
                pitch: h.pitch,
                velocity: h.velocity,
                ..Note::default()
            },
        })
    }

    /// Puts a finished note of the take in the ring.
    fn push(&mut self, note: RecordedNote) {
        let at = self.total as usize % RING;
        if let Some(slot) = self.ring.as_mut_slice().get_mut(at) {
            *slot = note;
        }
        self.total = self.total.wrapping_add(1);
    }
}

/// A shift in microseconds as whole samples at `rate`, clamped to
/// [`limits::MAX_RECORD_SHIFT_US`] either way; 0 for a rate that is not a positive number.
fn shift_samples(shift_us: i32, rate: f64) -> i64 {
    let us = shift_us.clamp(-limits::MAX_RECORD_SHIFT_US, limits::MAX_RECORD_SHIFT_US);
    let samples = (f64::from(us) * rate / 1_000_000.0).round();
    if samples.is_finite() && rate > 0.0 {
        // `as` saturates: at most 2 s of samples at any finite rate.
        samples as i64
    } else {
        0
    }
}

/// [`Recorder::sample_of`], a stamp behind the block never placed before `floor`.
fn usable(at: Option<u64>, clock: RecordClock, floor: u64) -> Option<u64> {
    let at = at?;
    let start = clock.sample;
    let bound = MAX_STAMP_DISTANCE_SECONDS * clock.rate;
    if at >= start {
        return ((at - start) as f64 <= bound).then_some(at);
    }
    if (start - at) as f64 > bound {
        return None;
    }
    Some(at.max(floor).max(clock.anchors.earliest()))
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
