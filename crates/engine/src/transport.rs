//! The musical clock of the engine (TR-1): play, stop, locate, tempo, loop, and the
//! conversion between the playhead in ticks and the samples of each audio block.
//!
//! [`Transport`] lives on the audio side. `Engine::apply` writes the orders into the
//! [`State`] (`playback`: playing, cue, loop; `transport`: tempo, signature); once per
//! block the render calls [`Transport::begin_block`], pulls [`Segment`]s with
//! [`Transport::next_segment`] until it returns `None`, then calls
//! [`Transport::end_block`]. Nothing here allocates, locks, logs or panics.
//!
//! # What a segment is
//!
//! A block is cut in segments at every point where the playhead jumps (the loop's end).
//! A [`Segment`] says: the frames `[offset, offset + frames)` of the block play the
//! musical ticks `[from, to)`. An event at tick `t` of that range is placed at
//! [`Segment::offset_of`]`(t)`, the sample that contains its fractional position.
//!
//! `from` of a segment is the `to` of the previous one (the clock keeps the first tick not
//! yet delivered), so every tick is delivered exactly once, in order, whatever the block
//! sizes: no gap and no duplicate at a block boundary, at a tempo change or at the loop
//! seam. A `Play` or a `Seek` restarts the delivery at the cue tick.
//!
//! # No drift
//!
//! The position at the absolute sample `n` is a pure function of `n` and of an anchor
//! (tick, sample), never an accumulation: `anchor_tick + (n - anchor_sample) /
//! samples_per_tick`. A division by the exact samples per tick, so at 120 BPM the position
//! is exact at 44.1, 48 and 96 kHz (22.96875, 25 and 50 samples per tick). The anchor moves
//! only at a `Play`, a `Seek`, a tempo change (the current position becomes the anchor:
//! no jump) and a loop wrap (the loop start is anchored at the fractional sample where the
//! playhead reached the loop end, so the seam keeps its sub-sample phase and the loop is
//! exactly its length in samples over any number of wraps).
//!
//! Orders take effect at the start of the block that applies them (module doc of
//! `crate::msg`), including a tempo change.
//!
//! # Placement and loop rules
//!
//! An event is placed at the *floor* of its fractional sample (the sample that contains
//! it), not at `Tempo::tick_to_sample_rounded`: ticks are assigned to a segment by
//! `position < end`, which only floor keeps consistent. The first tick after a loop wrap is
//! clamped into the first sample of its segment (up to one sample late).
//!
//! Whether the playhead is inside the loop is decided once (at a `Play`, a `Seek` or a
//! loop change), not from the computed position of each block: a wrap on a block boundary
//! never lets it escape. A playhead at or past the loop end when it is decided plays on.
//! The clock is built with the sample rate of its channel (`Transport::new`) and rebuilt
//! with it. It is distinct from `crate::device::Transport`, the view devices read; the
//! position of a segment for `BlockCtx` is `Segment::position_at` its first frame.
//!
//! # Saturation
//!
//! A position beyond the `i64` ticks saturates; a loop shorter than one sample is ignored
//! (it guarantees at most `frames + 1` wraps per block). The clock follows the state it is
//! given: a `State` whose `playing` is false stops it (a `Reset` does).

use crate::engine::State;
use crate::msg::TransportState;
use crate::time::{Frame, LoopRegion, SampleRate, Tempo, Tick, TimeSignature};

/// A run of frames of the block that plays a contiguous range of ticks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Segment {
    /// First frame of the segment inside the block.
    pub offset: usize,
    /// Frames in the segment.
    pub frames: usize,
    /// First tick whose event falls in the segment.
    pub from: Tick,
    /// The tick after the last one (exclusive).
    pub to: Tick,
    origin_tick: f64,
    /// Fractional sample, relative to the block start, where `origin_tick` is played.
    origin_sample: f64,
    samples_per_tick: f64,
}

impl Segment {
    /// The sample of the block that contains the event at `tick`: the floor of its
    /// fractional position, clamped to the segment (a tick outside `[from, to)` lands on
    /// its first or last frame).
    pub fn offset_of(&self, tick: Tick) -> usize {
        let at = self.origin_sample + (tick.0 as f64 - self.origin_tick) * self.samples_per_tick;
        let last = self.offset + self.frames.saturating_sub(1);
        // `as` saturates and sends NaN to zero.
        (at.floor() as usize).clamp(self.offset, last)
    }

    /// The playhead, in fractional ticks, at frame `offset` of the block.
    pub fn position_at(&self, offset: usize) -> f64 {
        self.origin_tick + (offset as f64 - self.origin_sample) / self.samples_per_tick
    }

    /// The fractional position of `tick`, in samples from the block start (not clamped).
    pub fn sample_of(&self, tick: Tick) -> f64 {
        self.origin_sample + (tick.0 as f64 - self.origin_tick) * self.samples_per_tick
    }
}

/// A position as bars, beats and ticks, all zero-based (the UI adds one to the first two).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BarPosition {
    pub bar: i64,
    pub beat: u32,
    pub tick: u32,
}

/// The bar, beat and tick of `tick` in `signature` (the grid is anchored at tick 0; a
/// negative tick gives a negative bar). In compound meters one beat is the denominator's
/// note value (`TimeSignature::ticks_per_beat`).
pub fn bar_position(tick: Tick, signature: TimeSignature) -> BarPosition {
    let bar_len = signature.ticks_per_bar().0.max(1);
    let beat_len = signature.ticks_per_beat().0.max(1);
    let in_bar = tick.0.rem_euclid(bar_len);
    BarPosition {
        bar: tick.0.div_euclid(bar_len),
        beat: (in_bar / beat_len) as u32,
        tick: (in_bar % beat_len) as u32,
    }
}

/// The audio side's musical clock. See the module documentation.
#[derive(Clone, Debug)]
pub struct Transport {
    rate: f64,
    tempo: Tempo,
    samples_per_tick: f64,
    running: bool,
    recording: bool,
    /// The playhead while stopped, in ticks.
    frozen: f64,
    anchor_tick: f64,
    /// Absolute sample (fractional) where `anchor_tick` is played.
    anchor_sample: f64,
    /// The first tick not yet delivered.
    next_tick: Tick,
    /// Absolute sample index of the start of the block in progress (or of the next one).
    total: u64,
    frames: usize,
    cursor: usize,
    open: bool,
    loop_region: Option<LoopRegion>,
    /// The loop the playhead is inside of: decided once, when the clock starts, a cue
    /// moves it or the loop changes, never again from a computed position (a wrap sample
    /// on a block boundary must not let the playhead escape).
    loop_armed: bool,
    /// Whether the loop was in force (long enough) when `loop_armed` was last decided.
    loop_active: bool,
    /// Wraps taken in the block in progress (bounded by `frames + 1`).
    wraps: usize,
}

impl Transport {
    /// A stopped clock at tick 0, at the default tempo.
    pub fn new(rate: SampleRate) -> Self {
        let tempo = Tempo::default();
        let rate = f64::from(rate.hz());
        Transport {
            rate,
            tempo,
            samples_per_tick: samples_per_tick(tempo, rate),
            running: false,
            recording: false,
            frozen: 0.0,
            anchor_tick: 0.0,
            anchor_sample: 0.0,
            next_tick: Tick::ZERO,
            total: 0,
            frames: 0,
            cursor: 0,
            open: false,
            loop_region: None,
            loop_armed: false,
            loop_active: false,
            wraps: 0,
        }
    }

    /// Whether the clock is running.
    pub fn is_running(&self) -> bool {
        self.running
    }

    /// Stopped, playing or recording, for the feedback.
    pub fn state(&self) -> TransportState {
        match (self.running, self.recording) {
            (false, _) => TransportState::Stopped,
            (true, false) => TransportState::Playing,
            (true, true) => TransportState::Recording,
        }
    }

    /// Samples rendered since the construction (the start of the block in progress, or of
    /// the next block once [`Transport::end_block`] ran).
    pub fn total_samples(&self) -> u64 {
        self.total
    }

    /// The playhead in fractional ticks: at the start of the block in progress, or of the
    /// next block once [`Transport::end_block`] ran.
    pub fn position(&self) -> f64 {
        if self.running {
            self.position_at(self.total as f64)
        } else {
            self.frozen
        }
    }

    /// The playhead tick (floor of [`Transport::position`], saturating).
    pub fn playhead_tick(&self) -> Tick {
        Tick(self.position().floor() as i64)
    }

    /// The playhead as a sample count from tick 0 at the current tempo, rounded, never
    /// negative (the `playhead_frame` of the feedback).
    pub fn playhead_frame(&self) -> Frame {
        let samples = (self.position() * self.samples_per_tick).round();
        Frame(if samples > 0.0 { samples as u64 } else { 0 })
    }

    /// Starts a block of `frames` frames: reads the orders `Engine::apply` left in
    /// `state` (taking the cue) and moves the clock accordingly. A block still open is
    /// ended first.
    pub fn begin_block(&mut self, state: &mut State, frames: usize) {
        self.end_block();
        let total = self.total as f64;
        let was_running = self.running;
        // A tempo change keeps the current position: re-anchor first.
        let tempo = state.transport.tempo;
        if tempo != self.tempo {
            if self.running {
                self.anchor_tick = self.position_at(total);
                self.anchor_sample = total;
            }
            self.tempo = tempo;
            self.samples_per_tick = samples_per_tick(tempo, self.rate);
        }
        let playback = &mut state.playback;
        let cue = playback.cue.take();
        self.recording = playback.recording;
        let loop_changed = self.loop_region != playback.loop_region;
        self.loop_region = playback.loop_region;
        if playback.playing {
            match cue {
                Some(tick) => self.start_at(tick.0 as f64, tick),
                None if !self.running => {
                    let at = self.frozen;
                    self.start_at(at, Tick(at.ceil() as i64));
                }
                None => {}
            }
        } else {
            if self.running {
                self.frozen = self.position_at(total);
                self.running = false;
            }
            if let Some(tick) = cue {
                self.frozen = tick.0 as f64;
                self.next_tick = tick;
            }
        }
        let loop_active = self.active_loop().is_some();
        if loop_changed || cue.is_some() || !was_running || loop_active != self.loop_active {
            self.loop_armed = self.running
                && self
                    .active_loop()
                    .is_some_and(|r| self.position_at(total) < r.end().0 as f64);
        }
        self.loop_active = loop_active;
        self.frames = frames;
        self.cursor = 0;
        self.wraps = 0;
        self.open = true;
    }

    /// The next segment of the block, or `None` when the block is done or the clock is
    /// stopped.
    pub fn next_segment(&mut self) -> Option<Segment> {
        while self.open && self.running && self.cursor < self.frames {
            let total = self.total as f64;
            let block_end = total + self.frames as f64;
            let spt = self.samples_per_tick;
            let offset = self.cursor;
            let from = self.next_tick;
            let origin_tick = self.anchor_tick;
            let origin_sample = self.anchor_sample - total;

            // Where the playhead reaches the end of the loop, in absolute samples.
            let wrap = self
                .active_loop()
                .filter(|_| self.loop_armed)
                .and_then(|region| {
                    let end = region.end().0 as f64;
                    let reaches = self.anchor_sample + (end - self.anchor_tick) * spt;
                    (reaches <= block_end).then_some((region, reaches))
                });

            // A wrap that would not advance (an `f64` loop that rounds to nothing) is
            // refused after `frames + 1` of them: the block always ends.
            let wrap = wrap.filter(|_| {
                self.wraps += 1;
                self.wraps <= self.frames + 1
            });
            if wrap.is_none() && self.wraps > self.frames + 1 {
                self.loop_armed = false;
            }
            let segment = if let Some((region, reaches)) = wrap {
                // The frames that start before the wrap play the end of the loop; the
                // loop start is anchored at the fractional wrap sample.
                let stop = ((reaches - total).ceil() as usize).clamp(offset, self.frames);
                let to = Tick(region.end().0.max(from.0));
                self.anchor_tick = region.start().0 as f64;
                self.anchor_sample = reaches;
                self.next_tick = region.start();
                self.cursor = stop;
                Segment {
                    offset,
                    frames: stop - offset,
                    from,
                    to,
                    origin_tick,
                    origin_sample,
                    samples_per_tick: spt,
                }
            } else {
                let to = Tick(self.position_at(block_end).ceil() as i64);
                let to = Tick(to.0.max(from.0));
                self.next_tick = to;
                self.cursor = self.frames;
                Segment {
                    offset,
                    frames: self.frames - offset,
                    from,
                    to,
                    origin_tick,
                    origin_sample,
                    samples_per_tick: spt,
                }
            };
            if segment.frames > 0 || segment.from < segment.to {
                return Some(segment);
            }
        }
        None
    }

    /// Ends the block: drains the segments not taken (so that loop wraps are applied) and
    /// moves the clock to the start of the next block.
    pub fn end_block(&mut self) {
        if !self.open {
            return;
        }
        while self.next_segment().is_some() {}
        self.total = self.total.saturating_add(self.frames as u64);
        self.frames = 0;
        self.cursor = 0;
        self.open = false;
    }

    /// The loop that is in force: none when shorter than one sample at the current tempo.
    fn active_loop(&self) -> Option<LoopRegion> {
        let region = self.loop_region?;
        let len = region.end().0 as f64 - region.start().0 as f64;
        (len * self.samples_per_tick >= 1.0).then_some(region)
    }

    fn position_at(&self, sample: f64) -> f64 {
        self.anchor_tick + (sample - self.anchor_sample) / self.samples_per_tick
    }

    fn start_at(&mut self, tick: f64, next: Tick) {
        self.running = true;
        self.anchor_tick = tick;
        self.anchor_sample = self.total as f64;
        self.next_tick = next;
    }
}

fn samples_per_tick(tempo: Tempo, rate: f64) -> f64 {
    let spt = 60.0 * rate / (tempo.bpm() * crate::time::PPQ as f64);
    if spt.is_finite() && spt > 0.0 {
        spt
    } else {
        1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bar_position_in_simple_and_compound_meters() {
        let four = TimeSignature::COMMON;
        assert_eq!(
            bar_position(Tick(0), four),
            BarPosition {
                bar: 0,
                beat: 0,
                tick: 0
            }
        );
        assert_eq!(
            bar_position(Tick(960 * 5 + 7), four),
            BarPosition {
                bar: 1,
                beat: 1,
                tick: 7
            }
        );
        let six_eight = TimeSignature::new(6, 8).unwrap();
        assert_eq!(
            bar_position(Tick(480 * 7 + 1), six_eight),
            BarPosition {
                bar: 1,
                beat: 1,
                tick: 1
            }
        );
        assert_eq!(
            bar_position(Tick(-1), four),
            BarPosition {
                bar: -1,
                beat: 3,
                tick: 959
            }
        );
        assert_eq!(bar_position(Tick(i64::MAX), four).bar, i64::MAX / 3840);
    }
}

/// A scenario for the native/WASM parity test (`--features testing`): odd block sizes,
/// a loop, tempos that are not dyadic ratios, a seek, a stop and a play, at 44.1 kHz.
/// The clock is `f64` arithmetic, `ceil` and `floor`: the output is compared bit for bit.
#[cfg(feature = "testing")]
pub mod testing {
    use super::*;

    /// Sample rate of the scenario.
    pub const SCENARIO_RATE: u32 = 44_100;
    /// Blocks rendered by the scenario.
    pub const SCENARIO_BLOCKS: usize = 600;

    /// Runs the scenario and returns, per block, the playhead tick, the bits of the
    /// position, the number of segments, then for each segment its offset, frames, `from`,
    /// `to` and the offsets of its first and last tick.
    pub fn scenario() -> Vec<i64> {
        const SIZES: [usize; 7] = [64, 127, 480, 512, 1021, 33, 2048];
        let Some(rate) = SampleRate::new(SCENARIO_RATE) else {
            return Vec::new();
        };
        let Ok(mut state) = State::try_new() else {
            return Vec::new();
        };
        let mut clock = Transport::new(rate);
        let mut out = Vec::new();
        state.playback.loop_region = LoopRegion::new(Tick(960), Tick(5000));
        state.playback.playing = true;
        state.playback.cue = Some(Tick(0));
        for block in 0..SCENARIO_BLOCKS {
            match block {
                100 => state.transport.tempo = Tempo::clamped(97.3),
                200 => state.playback.cue = Some(Tick(1234)),
                300 => state.transport.tempo = Tempo::clamped(333.3),
                400 => state.playback.playing = false,
                450 => {
                    state.playback.playing = true;
                    state.playback.cue = Some(Tick(3000));
                }
                500 => state.playback.loop_region = None,
                _ => {}
            }
            clock.begin_block(&mut state, SIZES[block % SIZES.len()]);
            let at = out.len();
            out.extend([clock.playhead_tick().0, 0, 0]);
            let mut count = 0;
            while let Some(seg) = clock.next_segment() {
                count += 1;
                out.extend([
                    seg.offset as i64,
                    seg.frames as i64,
                    seg.from.0,
                    seg.to.0,
                    seg.offset_of(seg.from) as i64,
                    seg.offset_of(Tick(seg.to.0.saturating_sub(1))) as i64,
                ]);
            }
            clock.end_block();
            out[at + 1] = clock.position().to_bits() as i64;
            out[at + 2] = count;
        }
        out
    }
}
