//! Test devices of the contract (feature `testing`): an instrument ([`TestTone`]), an
//! audio effect ([`TestGain`]) and a MIDI effect ([`TestTranspose`]), and a host that
//! chains them ([`Chain`]) the way the contract says (MIDI effects, instrument, effects).
//! The contract tests render them offline and assert on the signal; the native/WASM parity
//! test renders [`scenario`] in both builds. They are not built-in devices: not in
//! [`super::builtin_descriptors`], not exported in the schema.

use core::f32::consts::TAU;

use super::{
    AudioIo, BlockCtx, Device, DeviceDescriptor, DeviceRole, DeviceSetup, Event, EventBuffer,
    MidiOutput, ParamDescriptor, Scale, Transport, Unit, VoiceId, clamp_tune_cents, event_frame,
};
use crate::msg::{DeviceKind, MsgError, ParamId, try_box};

// ---------------------------------------------------------------- shared pieces

/// A linear ramp as the contract defines smoothing: the value at frame `k + i` after a
/// target set at frame `k` is `start + (target − start) · min(i, n) / n`.
#[derive(Clone, Copy, Debug)]
pub struct Ramp {
    start: f32,
    target: f32,
    frames: u32,
    done: u32,
}

impl Ramp {
    pub const fn new(value: f32) -> Self {
        Self {
            start: value,
            target: value,
            frames: 0,
            done: 0,
        }
    }

    /// The value at the next frame.
    pub fn current(&self) -> f32 {
        if self.done >= self.frames {
            self.target
        } else {
            self.start + (self.target - self.start) * (self.done as f32 / self.frames as f32)
        }
    }

    /// Starts a ramp from the current value to `target` over `frames` frames (0: a jump).
    pub fn set(&mut self, target: f32, frames: u32) {
        self.start = self.current();
        self.target = target;
        self.frames = frames;
        self.done = 0;
    }

    /// Ends the ramp at its target.
    pub fn snap(&mut self) {
        self.start = self.target;
        self.frames = 0;
        self.done = 0;
    }

    /// The value of this frame, then moves one frame on.
    pub fn next_value(&mut self) -> f32 {
        let v = self.current();
        if self.done < self.frames {
            self.done += 1;
        }
        v
    }
}

/// Samples below this magnitude are flushed to zero (denormal guard).
const DENORMAL_FLOOR: f32 = 1e-20;

fn flush(x: f32) -> f32 {
    if x.abs() < DENORMAL_FLOOR { 0.0 } else { x }
}

/// Renders the frames between events and applies each event at its frame, under the
/// contract's tolerance rules ([`event_frame`]).
fn split<D: ?Sized>(
    device: &mut D,
    frames: usize,
    events: &[Event],
    mut render: impl FnMut(&mut D, usize, usize),
    mut apply: impl FnMut(&mut D, &Event),
) {
    let mut cursor = 0;
    for event in events {
        let at = event_frame(event.offset(), cursor, frames);
        if at > cursor {
            render(device, cursor, at);
            cursor = at;
        }
        apply(device, event);
    }
    if frames > cursor {
        render(device, cursor, frames);
    }
}

fn smoothed(desc: &DeviceDescriptor, id: ParamId) -> &'static ParamDescriptor {
    // The descriptors below are static and declare every id they look up.
    match desc.param(id) {
        Some(p) => p,
        None => &desc.params[0],
    }
}

// ---------------------------------------------------------------- the instrument

/// Parameters of [`TestTone`].
pub mod tone {
    use crate::msg::ParamId;
    pub const GAIN: ParamId = ParamId(0);
    pub const OCTAVE: ParamId = ParamId(1);
}

/// Descriptor of [`TestTone`].
pub static TONE: DeviceDescriptor = DeviceDescriptor {
    kind: "faulk.test_tone",
    kind_id: DeviceKind(0xFFFF_0001),
    version: 1,
    name: "Test Tone",
    role: DeviceRole::Instrument,
    memory_bytes: size_of::<TestTone>() as u64,
    midi_output: None,
    params: &[
        ParamDescriptor {
            id: tone::GAIN,
            key: "gain",
            unit: Unit::None,
            min: 0.0,
            max: 1.0,
            default: 0.5,
            scale: Scale::Linear,
            step: 0.0,
            smoothing_seconds: 0.02,
            values: &[],
        },
        ParamDescriptor {
            id: tone::OCTAVE,
            key: "octave",
            unit: Unit::None,
            min: -2.0,
            max: 2.0,
            default: 0.0,
            scale: Scale::Linear,
            step: 1.0,
            smoothing_seconds: 0.0,
            values: &[],
        },
    ],
};

/// Voices of [`TestTone`].
pub const TONE_VOICES: usize = 8;

#[derive(Clone, Copy, Debug, Default)]
struct ToneVoice {
    active: bool,
    voice: VoiceId,
    /// Cycles per frame at octave 0.
    increment: f64,
    phase: f64,
    level: f32,
    started: u64,
}

/// An instrument: one cosine per note (the first sample of a note is its peak, so an onset
/// is exact), hard gated, pitch from `key + tune_cents`, a smoothed `gain` and a stepped
/// `octave`.
#[derive(Clone, Debug)]
pub struct TestTone {
    sample_rate: f32,
    voices: [ToneVoice; TONE_VOICES],
    gain: Ramp,
    octave: f32,
    counter: u64,
}

impl TestTone {
    pub fn new(setup: &DeviceSetup) -> Self {
        Self {
            sample_rate: sane_rate(setup.sample_rate),
            voices: [ToneVoice::default(); TONE_VOICES],
            gain: Ramp::new(TONE.params[0].default as f32),
            octave: TONE.params[1].default as f32,
            counter: 0,
        }
    }

    pub fn build(setup: &DeviceSetup) -> Result<Box<dyn Device>, MsgError> {
        Ok(try_box(Self::new(setup))?)
    }

    /// Notes sounding.
    pub fn active_voices(&self) -> usize {
        self.voices.iter().filter(|v| v.active).count()
    }

    fn apply(&mut self, event: &Event) {
        match *event {
            Event::NoteOn {
                voice,
                key,
                velocity,
                tune_cents,
                ..
            } => {
                let velocity = if velocity.is_finite() {
                    velocity.clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let pitch =
                    f64::from(key.min(127)) + f64::from(clamp_tune_cents(tune_cents)) / 100.0;
                let freq = 440.0 * ((pitch - 69.0) / 12.0).exp2();
                self.counter += 1;
                let slot = self
                    .voices
                    .iter()
                    .position(|v| v.active && v.voice == voice)
                    .or_else(|| self.voices.iter().position(|v| !v.active))
                    .unwrap_or_else(|| {
                        // Steal the oldest note.
                        let mut oldest = 0;
                        for (i, v) in self.voices.iter().enumerate() {
                            if v.started < self.voices[oldest].started {
                                oldest = i;
                            }
                        }
                        oldest
                    });
                self.voices[slot] = ToneVoice {
                    active: true,
                    voice,
                    increment: freq / f64::from(self.sample_rate),
                    phase: 0.0,
                    level: velocity,
                    started: self.counter,
                };
            }
            Event::NoteOff { voice, .. } => {
                for v in self.voices.iter_mut().filter(|v| v.voice == voice) {
                    v.active = false;
                }
            }
            Event::AllNotesOff { .. } => {
                for v in &mut self.voices {
                    v.active = false;
                }
            }
            Event::ParamValue { id, value, .. } => self.set_param(id, value),
        }
    }

    fn render(&mut self, io: &mut AudioIo<'_>, from: usize, to: usize) {
        let octave = f64::from(self.octave).exp2();
        let (left, right) = (&mut io.left[from..to], &mut io.right[from..to]);
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            let gain = self.gain.next_value();
            let mut sum = 0.0f32;
            for v in self.voices.iter_mut().filter(|v| v.active) {
                sum += (TAU * v.phase as f32).cos() * v.level;
                v.phase = (v.phase + v.increment * octave).fract();
            }
            let s = flush(sum * gain * 0.25);
            *l = s;
            *r = s;
        }
    }
}

impl Device for TestTone {
    fn descriptor(&self) -> &'static DeviceDescriptor {
        &TONE
    }

    fn process(&mut self, ctx: &BlockCtx, events: &[Event], io: &mut AudioIo<'_>) {
        let frames = block_frames(ctx, io);
        split(
            self,
            frames,
            events,
            |d, from, to| d.render(io, from, to),
            |d, e| d.apply(e),
        );
    }

    fn set_param(&mut self, id: ParamId, value: f32) {
        let Some(desc) = TONE.param(id) else { return };
        let Some(value) = desc.sanitize(value) else {
            return;
        };
        if id == tone::GAIN {
            let frames = smoothed(&TONE, id).smoothing_frames(self.sample_rate);
            self.gain.set(value, frames);
        } else if id == tone::OCTAVE {
            self.octave = value;
        }
    }

    fn reset(&mut self) {
        for v in &mut self.voices {
            *v = ToneVoice::default();
        }
        self.gain.snap();
    }
}

// ---------------------------------------------------------------- the effect

/// Parameters of [`TestGain`].
pub mod gain {
    use crate::msg::ParamId;
    pub const GAIN: ParamId = ParamId(0);
    pub const MUTE: ParamId = ParamId(1);
}

/// Descriptor of [`TestGain`].
pub static GAIN: DeviceDescriptor = DeviceDescriptor {
    kind: "faulk.test_gain",
    kind_id: DeviceKind(0xFFFF_0002),
    version: 1,
    name: "Test Gain",
    role: DeviceRole::Effect,
    memory_bytes: size_of::<TestGain>() as u64,
    midi_output: None,
    params: &[
        ParamDescriptor {
            id: gain::GAIN,
            key: "gain",
            unit: Unit::None,
            min: 0.0,
            max: 2.0,
            default: 1.0,
            scale: Scale::Linear,
            step: 0.0,
            smoothing_seconds: 0.01,
            values: &[],
        },
        ParamDescriptor {
            id: gain::MUTE,
            key: "mute",
            unit: Unit::None,
            min: 0.0,
            max: 1.0,
            default: 0.0,
            scale: Scale::Linear,
            step: 1.0,
            smoothing_seconds: 0.0,
            values: &["off", "on"],
        },
    ],
};

/// An audio effect: multiplies both channels by a smoothed `gain`, silenced by a stepped
/// `mute`.
#[derive(Clone, Debug)]
pub struct TestGain {
    sample_rate: f32,
    gain: Ramp,
    muted: bool,
}

impl TestGain {
    pub fn new(setup: &DeviceSetup) -> Self {
        Self {
            sample_rate: sane_rate(setup.sample_rate),
            gain: Ramp::new(GAIN.params[0].default as f32),
            muted: GAIN.params[1].default != 0.0,
        }
    }

    pub fn build(setup: &DeviceSetup) -> Result<Box<dyn Device>, MsgError> {
        Ok(try_box(Self::new(setup))?)
    }

    fn render(&mut self, io: &mut AudioIo<'_>, from: usize, to: usize) {
        let (left, right) = (&mut io.left[from..to], &mut io.right[from..to]);
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            let g = if self.muted {
                // The ramp still moves while muted.
                self.gain.next_value();
                0.0
            } else {
                self.gain.next_value()
            };
            *l = flush(sanitize_sample(*l) * g);
            *r = flush(sanitize_sample(*r) * g);
        }
    }
}

/// An input sample as an effect reads it: a non-finite one is silence.
fn sanitize_sample(x: f32) -> f32 {
    if x.is_finite() { x } else { 0.0 }
}

impl Device for TestGain {
    fn descriptor(&self) -> &'static DeviceDescriptor {
        &GAIN
    }

    fn process(&mut self, ctx: &BlockCtx, events: &[Event], io: &mut AudioIo<'_>) {
        let frames = block_frames(ctx, io);
        split(
            self,
            frames,
            events,
            |d, from, to| d.render(io, from, to),
            |d, e| {
                if let Event::ParamValue { id, value, .. } = *e {
                    d.set_param(id, value);
                }
            },
        );
    }

    fn set_param(&mut self, id: ParamId, value: f32) {
        let Some(desc) = GAIN.param(id) else { return };
        let Some(value) = desc.sanitize(value) else {
            return;
        };
        if id == gain::GAIN {
            self.gain
                .set(value, desc.smoothing_frames(self.sample_rate));
        } else if id == gain::MUTE {
            self.muted = value != 0.0;
        }
    }

    fn reset(&mut self) {
        self.gain.snap();
    }
}

// ---------------------------------------------------------------- the MIDI effect

/// Parameters of [`TestTranspose`].
pub mod transpose {
    use crate::msg::ParamId;
    pub const SEMITONES: ParamId = ParamId(0);
    pub const CENTS: ParamId = ParamId(1);
}

/// Descriptor of [`TestTranspose`].
pub static TRANSPOSE: DeviceDescriptor = DeviceDescriptor {
    kind: "faulk.test_transpose",
    kind_id: DeviceKind(0xFFFF_0003),
    version: 1,
    name: "Test Transpose",
    role: DeviceRole::MidiEffect,
    memory_bytes: size_of::<TestTranspose>() as u64,
    midi_output: Some(MidiOutput {
        per_input: 1,
        per_block: 0,
    }),
    params: &[
        ParamDescriptor {
            id: transpose::SEMITONES,
            key: "semitones",
            unit: Unit::Semitones,
            min: -24.0,
            max: 24.0,
            default: 0.0,
            scale: Scale::Linear,
            step: 1.0,
            smoothing_seconds: 0.0,
            values: &[],
        },
        ParamDescriptor {
            id: transpose::CENTS,
            key: "cents",
            unit: Unit::Cents,
            min: -100.0,
            max: 100.0,
            default: 0.0,
            scale: Scale::Linear,
            step: 0.0,
            smoothing_seconds: 0.0,
            values: &[],
        },
    ],
};

/// A MIDI effect: transposes each note-on by `semitones` (key clamped to 0–127) and adds
/// `cents` to its fine tuning; note-offs and all-notes-off pass through with their voice.
/// A parameter applies to the notes that start at or after its offset.
#[derive(Clone, Debug)]
pub struct TestTranspose {
    semitones: i32,
    cents: f32,
}

impl TestTranspose {
    pub fn new(_setup: &DeviceSetup) -> Self {
        Self {
            semitones: 0,
            cents: 0.0,
        }
    }

    pub fn build(setup: &DeviceSetup) -> Result<Box<dyn Device>, MsgError> {
        Ok(try_box(Self::new(setup))?)
    }

    fn key(&self, key: u8) -> u8 {
        (i32::from(key.min(127)) + self.semitones).clamp(0, 127) as u8
    }
}

impl Device for TestTranspose {
    fn descriptor(&self) -> &'static DeviceDescriptor {
        &TRANSPOSE
    }

    fn process(&mut self, ctx: &BlockCtx, events: &[Event], io: &mut AudioIo<'_>) {
        let frames = block_frames(ctx, io);
        let mut cursor = 0;
        for event in events {
            let at = event_frame(event.offset(), cursor, frames);
            cursor = at;
            let at = at as u32;
            match *event {
                Event::NoteOn {
                    voice,
                    key,
                    velocity,
                    tune_cents,
                    ..
                } => {
                    io.events_out.push(Event::NoteOn {
                        offset: at,
                        voice,
                        key: self.key(key),
                        velocity,
                        tune_cents: clamp_tune_cents(clamp_tune_cents(tune_cents) + self.cents),
                    });
                }
                Event::NoteOff { voice, key, .. } => {
                    io.events_out.push(Event::NoteOff {
                        offset: at,
                        voice,
                        key: self.key(key),
                    });
                }
                Event::AllNotesOff { .. } => {
                    io.events_out.push(Event::AllNotesOff { offset: at });
                }
                Event::ParamValue { id, value, .. } => self.set_param(id, value),
            }
        }
    }

    fn set_param(&mut self, id: ParamId, value: f32) {
        let Some(desc) = TRANSPOSE.param(id) else {
            return;
        };
        let Some(value) = desc.sanitize(value) else {
            return;
        };
        if id == transpose::SEMITONES {
            self.semitones = value as i32;
        } else if id == transpose::CENTS {
            self.cents = value;
        }
    }

    fn reset(&mut self) {}
}

fn sane_rate(rate: f32) -> f32 {
    if rate.is_finite() && rate >= 8_000.0 {
        rate.min(super::MAX_SAMPLE_RATE)
    } else {
        48_000.0
    }
}

/// The frames of a block: `ctx.frames`, never more than the buffers hold.
fn block_frames(ctx: &BlockCtx, io: &AudioIo<'_>) -> usize {
    (ctx.frames as usize).min(io.left.len()).min(io.right.len())
}

// ---------------------------------------------------------------- a host for the tests

/// Which device of a [`Chain`] a scheduled event goes to. Notes always go to the head of
/// the chain (the MIDI effect).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Transpose,
    Tone,
    Gain,
}

/// An event at an absolute frame (its own offset is ignored).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scheduled {
    pub frame: u64,
    pub target: Target,
    pub event: Event,
}

/// Capacity of each per-device input buffer of a [`Chain`].
pub const CHAIN_INPUT_CAPACITY: usize = 4096;

/// A track's chain as the contract orders it: `TestTranspose` → `TestTone` → `TestGain`,
/// with the host's routing (MIDI effect output merged with the instrument's own parameter
/// values, by offset and [`Event::rank`]). Built off the audio path; [`Chain::render`]
/// never allocates.
pub struct Chain {
    pub transpose: TestTranspose,
    pub tone: TestTone,
    pub gain: TestGain,
    sample_rate: f32,
    to_transpose: EventBuffer,
    to_tone: EventBuffer,
    to_gain: EventBuffer,
    midi_out: EventBuffer,
    merged: EventBuffer,
    scratch: EventBuffer,
    left: Box<[f32]>,
    right: Box<[f32]>,
}

impl Chain {
    pub fn new(setup: &DeviceSetup) -> Result<Self, MsgError> {
        let buffer = |frames: usize| -> Result<Box<[f32]>, MsgError> {
            let mut v = Vec::new();
            v.try_reserve_exact(frames)
                .map_err(|_| MsgError::OutOfMemory { bytes: frames * 4 })?;
            v.resize(frames, 0.0);
            Ok(v.into_boxed_slice())
        };
        // Sized from the MIDI effect's declared bounds, so that it never fills.
        let midi_capacity = TRANSPOSE
            .midi_output
            .unwrap_or_default()
            .capacity(CHAIN_INPUT_CAPACITY);
        Ok(Self {
            transpose: TestTranspose::new(setup),
            tone: TestTone::new(setup),
            gain: TestGain::new(setup),
            sample_rate: sane_rate(setup.sample_rate),
            to_transpose: EventBuffer::try_with_capacity(CHAIN_INPUT_CAPACITY)?,
            to_tone: EventBuffer::try_with_capacity(CHAIN_INPUT_CAPACITY)?,
            to_gain: EventBuffer::try_with_capacity(CHAIN_INPUT_CAPACITY)?,
            midi_out: EventBuffer::try_with_capacity(midi_capacity)?,
            merged: EventBuffer::try_with_capacity(midi_capacity + CHAIN_INPUT_CAPACITY)?,
            scratch: EventBuffer::try_with_capacity(1)?,
            left: buffer(setup.max_block_frames.max(1))?,
            right: buffer(setup.max_block_frames.max(1))?,
        })
    }

    /// Renders `script` (sorted by frame) from frame 0 in blocks whose sizes cycle through
    /// `blocks` (each clamped to `1..=max_block_frames`), into `left` and `right` (same
    /// length). Returns the events the MIDI effect's output dropped (overflows).
    pub fn render(
        &mut self,
        script: &[Scheduled],
        blocks: &[usize],
        left: &mut [f32],
        right: &mut [f32],
    ) -> u32 {
        let total = left.len().min(right.len());
        let max = self.left.len();
        let mut start = 0usize;
        let mut next = 0usize;
        let mut overflowed = 0u32;
        let mut b = 0usize;
        while start < total {
            let size = blocks
                .get(b % blocks.len().max(1))
                .copied()
                .unwrap_or(128)
                .clamp(1, max)
                .min(total - start);
            b += 1;
            let end = start + size;
            for buf in [
                &mut self.to_transpose,
                &mut self.to_tone,
                &mut self.to_gain,
                &mut self.midi_out,
                &mut self.merged,
            ] {
                buf.clear();
            }
            while next < script.len() && (script[next].frame as usize) < end {
                let s = script[next];
                let offset = (s.frame as usize).saturating_sub(start) as u32;
                let event = s.event.at(offset);
                match (s.target, event.is_note()) {
                    (_, true) | (Target::Transpose, false) => self.to_transpose.push(event),
                    (Target::Tone, false) => self.to_tone.push(event),
                    (Target::Gain, false) => self.to_gain.push(event),
                };
                next += 1;
            }
            let ctx = BlockCtx {
                sample_rate: self.sample_rate,
                frames: size as u32,
                transport: Transport::default(),
            };
            let (l, r) = (&mut self.left[..size], &mut self.right[..size]);
            l.fill(0.0);
            r.fill(0.0);

            // The MIDI effect: notes out.
            let mut io = AudioIo {
                left: l,
                right: r,
                events_out: &mut self.midi_out,
            };
            self.transpose
                .process(&ctx, self.to_transpose.events(), &mut io);
            self.midi_out.finish();
            overflowed = overflowed.saturating_add(self.midi_out.overflowed());

            // The instrument: the MIDI effect's notes merged with its own parameters.
            merge(
                self.midi_out.events().iter().filter(|e| e.is_note()),
                self.to_tone.events().iter(),
                &mut self.merged,
            );
            self.scratch.clear();
            let mut io = AudioIo {
                left: l,
                right: r,
                events_out: &mut self.scratch,
            };
            self.tone.process(&ctx, self.merged.events(), &mut io);

            // The effect.
            self.scratch.clear();
            let mut io = AudioIo {
                left: l,
                right: r,
                events_out: &mut self.scratch,
            };
            self.gain.process(&ctx, self.to_gain.events(), &mut io);

            left[start..end].copy_from_slice(l);
            right[start..end].copy_from_slice(r);
            start = end;
        }
        overflowed
    }
}

/// Merges two event streams sorted by `(offset, rank)` into `out`, keeping that order.
pub fn merge<'a>(
    a: impl Iterator<Item = &'a Event>,
    b: impl Iterator<Item = &'a Event>,
    out: &mut EventBuffer,
) {
    let mut a = a.peekable();
    let mut b = b.peekable();
    loop {
        let take_a = match (a.peek(), b.peek()) {
            (Some(x), Some(y)) => (x.offset(), x.rank()) <= (y.offset(), y.rank()),
            (Some(_), None) => true,
            (None, Some(_)) => false,
            (None, None) => break,
        };
        let e = if take_a { a.next() } else { b.next() };
        if let Some(e) = e {
            out.push(*e);
        }
    }
}

/// Frames of [`scenario`].
pub const SCENARIO_FRAMES: usize = 24_000;

/// Sample rate of [`scenario`].
pub const SCENARIO_RATE: f32 = 48_000.0;

/// Block sizes [`scenario`] cycles through: odd sizes, the Web Audio quantum, the largest.
pub const SCENARIO_BLOCKS: [usize; 7] = [128, 1, 64, 517, 2048, 333, 7];

const fn note_on(frame: u64, voice: u32, key: u8, velocity: f32, tune_cents: f32) -> Scheduled {
    Scheduled {
        frame,
        target: Target::Transpose,
        event: Event::NoteOn {
            offset: 0,
            voice: VoiceId(voice),
            key,
            velocity,
            tune_cents,
        },
    }
}

const fn note_off(frame: u64, voice: u32, key: u8) -> Scheduled {
    Scheduled {
        frame,
        target: Target::Transpose,
        event: Event::NoteOff {
            offset: 0,
            voice: VoiceId(voice),
            key,
        },
    }
}

const fn param(frame: u64, target: Target, id: ParamId, value: f32) -> Scheduled {
    Scheduled {
        frame,
        target,
        event: Event::ParamValue {
            offset: 0,
            id,
            value,
        },
    }
}

/// A fixed performance through the whole chain: notes with fine tuning, transposition,
/// octave, smoothed gains and a mute, at odd frames.
pub const SCENARIO: [Scheduled; 16] = [
    note_on(101, 1, 60, 0.8, 0.0),
    param(1_000, Target::Tone, tone::GAIN, 1.0),
    note_on(2_345, 2, 64, 0.6, 25.0),
    param(3_001, Target::Transpose, transpose::SEMITONES, 7.0),
    param(3_001, Target::Transpose, transpose::CENTS, -30.0),
    note_on(4_444, 3, 67, 0.7, -10.0),
    param(6_007, Target::Gain, gain::GAIN, 0.25),
    note_off(7_000, 1, 60),
    param(8_191, Target::Tone, tone::OCTAVE, 1.0),
    param(9_999, Target::Gain, gain::MUTE, 1.0),
    param(10_500, Target::Gain, gain::MUTE, 0.0),
    param(12_000, Target::Gain, gain::GAIN, 1.5),
    note_off(15_000, 2, 64),
    note_on(16_383, 4, 72, 1.0, 50.0),
    note_off(20_000, 3, 67),
    note_off(23_000, 4, 72),
];

/// Renders [`SCENARIO`] with [`SCENARIO_BLOCKS`] into `out`: the left channel in its first
/// [`SCENARIO_FRAMES`] samples, the right one after. Returns the samples written (0 when
/// `out` is too short or the chain could not be built).
pub fn scenario(out: &mut [f32]) -> usize {
    if out.len() < 2 * SCENARIO_FRAMES {
        return 0;
    }
    let setup = DeviceSetup {
        sample_rate: SCENARIO_RATE,
        max_block_frames: crate::msg::limits::MAX_BLOCK_FRAMES,
    };
    let Ok(mut chain) = Chain::new(&setup) else {
        return 0;
    };
    let (left, rest) = out.split_at_mut(SCENARIO_FRAMES);
    chain.render(
        &SCENARIO,
        &SCENARIO_BLOCKS,
        left,
        &mut rest[..SCENARIO_FRAMES],
    );
    2 * SCENARIO_FRAMES
}

// ---------------------------------------------------------------- conformance

/// A rule of the contract a device broke under [`conformance`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Violation {
    pub rule: &'static str,
    /// Block number of the run, and the frame or event index in it.
    pub block: usize,
    pub at: usize,
}

/// Largest block [`conformance`] renders.
pub const CONFORMANCE_MAX_FRAMES: usize = 2048;

/// The conformance run every device passes (the test devices here; DV-1 and the next
/// device tasks run it on every factory of the registry): `blocks` blocks of pseudo-random
/// sizes from 0 to [`CONFORMANCE_MAX_FRAMES`] (seeded, reproducible), each with up to 64
/// events that break every host guarantee the contract tolerates (offsets beyond the block
/// and unsorted, unknown parameter ids, NaN, infinities, values far out of range, velocities
/// and tunings out of range), random `set_param` and `reset` calls between blocks, and a
/// noise input for effects with NaN, infinities, huge and subnormal samples in it. The output must stay finite, without denormals, and a MIDI
/// effect's output must stay sorted and inside the block. Allocates nothing: run it under
/// the allocation detector.
pub fn conformance(
    device: &mut dyn Device,
    sample_rate: f32,
    seed: u64,
    blocks: usize,
    events_out: &mut EventBuffer,
) -> Result<(), Violation> {
    let desc = device.descriptor();
    let mut rng = seed ^ 0x9E37_79B9_7F4A_7C15;
    let mut next = move || {
        // xorshift64*
        rng ^= rng >> 12;
        rng ^= rng << 25;
        rng ^= rng >> 27;
        rng.wrapping_mul(0x2545_F491_4F6C_DD1D)
    };
    let mut left = [0.0f32; CONFORMANCE_MAX_FRAMES];
    let mut right = [0.0f32; CONFORMANCE_MAX_FRAMES];
    let mut events = [Event::AllNotesOff { offset: 0 }; 64];
    const WILD: [f32; 8] = [
        f32::NAN,
        f32::INFINITY,
        f32::NEG_INFINITY,
        1e30,
        -1e30,
        0.0,
        -0.0,
        f32::MIN_POSITIVE / 2.0,
    ];
    for block in 0..blocks {
        let frames = match next() % 8 {
            0 => 0,
            1 => 1,
            2 => CONFORMANCE_MAX_FRAMES,
            _ => (next() % CONFORMANCE_MAX_FRAMES as u64) as usize,
        };
        let count = (next() % 64) as usize;
        for e in events.iter_mut().take(count) {
            let r = next();
            let offset = match r % 5 {
                0 => u32::MAX,
                1 => frames as u32,
                _ => (next() % (frames as u64 + 1)) as u32,
            };
            let value = |n: u64| -> f32 {
                if n.is_multiple_of(3) {
                    WILD[(n / 3 % 8) as usize]
                } else {
                    ((n >> 8) % 20_001) as f32 / 100.0 - 100.0
                }
            };
            let voice = VoiceId((next() % 12) as u32);
            *e = match (r >> 8) % 4 {
                0 => Event::NoteOn {
                    offset,
                    voice,
                    key: (next() % 256) as u8,
                    velocity: value(next()),
                    tune_cents: value(next()) * 100.0,
                },
                1 => Event::NoteOff {
                    offset,
                    voice,
                    key: (next() % 256) as u8,
                },
                2 if r.is_multiple_of(7) => Event::AllNotesOff { offset },
                _ => {
                    let id = if desc.params.is_empty() || r.is_multiple_of(6) {
                        ParamId((next() % 1000) as u32 + 1000)
                    } else {
                        desc.params[(next() % desc.params.len() as u64) as usize].id
                    };
                    Event::ParamValue {
                        offset,
                        id,
                        value: value(next()) * 1000.0,
                    }
                }
            };
        }
        // Mostly sorted, sometimes not: the guarantee a device must survive breaking.
        if !next().is_multiple_of(3) {
            events[..count].sort_unstable_by_key(|e| (e.offset(), e.rank()));
        }
        if next().is_multiple_of(4) && !desc.params.is_empty() {
            let p = &desc.params[(next() % desc.params.len() as u64) as usize];
            device.set_param(p.id, WILD[(next() % 8) as usize]);
            device.set_param(p.id, (p.min + (p.max - p.min) * 0.37) as f32);
        }
        if next().is_multiple_of(16) {
            device.reset();
        }
        for (i, (l, r)) in left.iter_mut().zip(right.iter_mut()).enumerate() {
            let noise = if desc.role != DeviceRole::Effect {
                0.0
            } else if next().is_multiple_of(97) {
                // Hostile input: NaN, infinities, huge, subnormal.
                WILD[(next() % 8) as usize]
            } else {
                ((next() % 2001) as f32 / 1000.0 - 1.0)
                    * if i.is_multiple_of(2) { 1.0 } else { 0.5 }
            };
            *l = noise;
            *r = noise;
        }
        events_out.clear();
        let ctx = BlockCtx {
            sample_rate,
            frames: frames as u32,
            transport: Transport::default(),
        };
        let mut io = AudioIo {
            left: &mut left[..frames],
            right: &mut right[..frames],
            events_out,
        };
        device.process(&ctx, &events[..count], &mut io);
        for (at, s) in left[..frames].iter().chain(&right[..frames]).enumerate() {
            if !s.is_finite() {
                return Err(Violation {
                    rule: "output is finite",
                    block,
                    at,
                });
            }
            if *s != 0.0 && s.abs() < f32::MIN_POSITIVE {
                return Err(Violation {
                    rule: "output has no denormals",
                    block,
                    at,
                });
            }
        }
        if desc.role == DeviceRole::MidiEffect {
            events_out.finish();
            let mut last = 0;
            for (at, e) in events_out.events().iter().enumerate() {
                if e.offset() < last {
                    return Err(Violation {
                        rule: "MIDI output is sorted",
                        block,
                        at,
                    });
                }
                if frames > 0 && e.offset() as usize >= frames {
                    return Err(Violation {
                        rule: "MIDI output is inside the block",
                        block,
                        at,
                    });
                }
                last = e.offset();
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- factories

/// The factory of each test device: the resolver the cycle tests give
/// [`crate::engine::channel`] (MO-1).
pub fn factory(kind: DeviceKind) -> Option<super::Factory> {
    if kind == TONE.kind_id {
        Some(TestTone::build)
    } else if kind == GAIN.kind_id {
        Some(TestGain::build)
    } else if kind == TRANSPOSE.kind_id {
        Some(TestTranspose::build)
    } else {
        None
    }
}
