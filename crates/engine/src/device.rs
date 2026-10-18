//! Device contract (CT-5, `docs/specs/02-decoupage-mvp.md` § 2.5).
//!
//! A **device** is a link of a track's chain: an instrument, an audio effect or a MIDI
//! effect. Every built-in device, every Workshop module (through its `ModuleInstance`
//! adapter) and every hosted plugin wrapper implements the [`Device`] trait and has a
//! static [`DeviceDescriptor`]. This module declares the contract; it holds no device: the
//! built-in devices and their registry are in [`crate::devices`] (DV-1: the synth,
//! [`crate::devices::osc_synth`]). The shared parameter smoother is AM-3's; routing the
//! events of a track and splitting blocks at loop wraps is MO-4's ([`crate::events`]).
//!
//! # The chain of a track
//!
//! In order: the **MIDI effects** (events in, events out), then the **instrument** (events
//! in, stereo audio out), then the **audio effects** (stereo audio in place). A MIDI effect
//! exists on MIDI tracks only: an audio track's instrument is the clip player, which plays
//! no notes.
//!
//! # Events
//!
//! [`Event`]s are dated by an offset in the block, in frames, so that a note, and a
//! parameter value written by automation (AM-2), land at the exact sample whatever the
//! block size. The host (MO-4) guarantees, for the slice a device receives:
//!
//! - offsets are sorted (non-decreasing) and below [`BlockCtx::frames`];
//! - at the same offset, parameter values come first, then the note events (`NoteOn`,
//!   `NoteOff`, `AllNotesOff`) **in the order they were issued** ([`Event::rank`]): a note
//!   starts with the values set at its offset; a note-off then a note-on is a retrigger; a
//!   note-on then a note-off of the same voice (a tap shorter than a block, a zero gate) is
//!   a note that ends at once, never a stuck one; a note-on issued before a release (CT-3's
//!   live overflow, an overflowed MIDI effect output) is released by it, and one issued
//!   after a transport stop still plays. A MIDI effect writes its notes in the order it
//!   means them; its output, sorted by offset, needs no other sorting;
//! - a device only receives what is addressed to it: its own parameters, and notes for a
//!   MIDI effect or an instrument (an audio effect receives no notes);
//! - a block never crosses a loop wrap or a seek: the host splits it, so the transport
//!   position moves linearly within a block.
//!
//! A device never panics when a guarantee is broken: an offset at or beyond the block is
//! taken as the last frame, an offset earlier than the previous event is taken as "now",
//! an unknown [`ParamId`] is ignored, a non-finite value is ignored, a value out of range is
//! clamped and a stepped value is rounded to its step.
//!
//! A note is identified by its [`VoiceId`], which the host allocates among the held notes
//! (the Atelier spec's `note_id`, § 1.3, voice (a)): a [`Event::NoteOff`] ends the note that
//! has the same voice id; its `key` is informational. The voice ids a MIDI effect writes are
//! its own namespace (unique among the notes it holds, whether it passes an input id on or
//! makes one up for a note it creates). The host never merges that output with another
//! stream: it replaces the track's stream for the next device, so two effects, or an
//! effect and the live input, never collide, and the ids reach the next device unchanged
//! (MO-4). The live input and a schedule share the track's input with separate ranges of
//! ids (`crate::events::SCHEDULE_FIRST_VOICE`). The pitch of a note is `key +
//! tune_cents / 100` semitones (fine tuning, issue #170).
//!
//! # Parameters and smoothing
//!
//! A parameter value is in the unit its [`ParamDescriptor`] declares (seconds, hertz,
//! decibels…), never normalized. Each parameter declares its **smoothing time**: with 0, a
//! value applies at its exact offset `k`; above 0, with `n =
//! round(smoothing_seconds × sample_rate)` ([`ParamDescriptor::smoothing_frames`]), the
//! value at frame `k + i` is `start + (target − start) · min(i, n) / n`, `start` being the
//! value at frame `k`: a linear ramp that reaches the target exactly `n` frames after the
//! event. A new value during a ramp starts a new ramp from where the value is. Stepped and
//! enumerated parameters are never smoothed.
//! [`Device::set_param`] (a value set by hand or by an agent, `RtMsg::ParamSet`) behaves as a
//! [`Event::ParamValue`] at offset 0 of the next block. [`Device::reset`] snaps every
//! smoothed value to its target.
//!
//! # Audio and real time
//!
//! [`Device::process`], [`Device::set_param`] and [`Device::reset`] run on the audio thread:
//! no allocation, no free, no lock, no I/O, no panic. Everything a device needs is
//! allocated when it is built, by its [`Factory`], in `prepare` (off the audio path), with
//! fallible allocations ([`crate::msg::try_box`], [`crate::msg::MsgError::OutOfMemory`]).
//! A change of sample rate builds a new instance. The initial state of a built-in device
//! (`ControlMsg::DeviceInsert::state`) is its parameter values: `prepare` builds the device,
//! calls [`Device::set_param`] for each value, then [`Device::reset`] (which snaps the
//! ramps), before the device reaches the audio thread; the byte encoding of that state is
//! MO-1's. A device declares the most memory it
//! takes ([`DeviceDescriptor::memory_bytes`]); the project adds the declared sizes and
//! refuses an insert beyond [`crate::msg::limits::ENGINE_MEMORY_BUDGET_BYTES`].

#[cfg(feature = "serde")]
use serde::Serialize;

use crate::msg::{DeviceKind, MsgError, ParamId, limits};
use crate::time::{Tempo, TimeSignature};

/// Largest fine tuning a note carries, in cents (one octave each way); beyond, clamped.
pub const MAX_TUNE_CENTS: f32 = 1200.0;

/// Parameters a device kind may declare (the project's `MAX_PARAMS_PER_DEVICE`).
pub const MAX_PARAMS_PER_DEVICE: usize = 10_000;

/// Longest parameter key or device kind, in bytes (the project's `MAX_KEY_LEN`).
pub const MAX_KEY_LEN: usize = 128;

/// Highest sample rate a device must support; [`DeviceDescriptor::memory_bytes`] is the
/// memory at this rate.
pub const MAX_SAMPLE_RATE: f32 = 192_000.0;

/// Longest smoothing time a parameter may declare, in seconds.
pub const MAX_SMOOTHING_SECONDS: f64 = 10.0;

// ---------------------------------------------------------------- roles and events

/// What a device does in a track's chain.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize), serde(rename_all = "snake_case"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum DeviceRole {
    /// Notes in, stereo audio out: the sound of the track. At most one per track.
    Instrument,
    /// Stereo audio in place, after the instrument.
    Effect,
    /// Notes in, notes out, before the instrument (MIDI tracks only).
    MidiEffect,
}

/// The identifier of a held note, allocated by the host among the notes held, on 24 bits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct VoiceId(u32);

impl VoiceId {
    /// Largest voice id (24 bits).
    pub const MAX: u32 = 0x00FF_FFFF;

    /// A voice id; `None` above [`VoiceId::MAX`].
    pub const fn new(id: u32) -> Option<Self> {
        if id <= Self::MAX {
            Some(Self(id))
        } else {
            None
        }
    }

    pub const fn get(self) -> u32 {
        self.0
    }
}

/// An event of a block, dated by its `offset` in frames from the start of the block.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    /// Starts a note. `velocity` is 0 to 1; the pitch is `key + tune_cents / 100`
    /// semitones, `tune_cents` clamped to [`MAX_TUNE_CENTS`] (non-finite: 0).
    NoteOn {
        offset: u32,
        voice: VoiceId,
        key: u8,
        velocity: f32,
        tune_cents: f32,
    },
    /// Ends the note of `voice` (`key` is informational).
    NoteOff {
        offset: u32,
        voice: VoiceId,
        key: u8,
    },
    /// Ends every note (transport stop, lost live events, an overflowed output).
    AllNotesOff { offset: u32 },
    /// A parameter value, in the unit its descriptor declares.
    ParamValue {
        offset: u32,
        id: ParamId,
        value: f32,
    },
}

impl Event {
    /// The offset of the event in its block, in frames.
    pub const fn offset(&self) -> u32 {
        match *self {
            Event::NoteOn { offset, .. }
            | Event::NoteOff { offset, .. }
            | Event::AllNotesOff { offset }
            | Event::ParamValue { offset, .. } => offset,
        }
    }

    /// The same event at another offset.
    pub const fn at(self, at: u32) -> Self {
        match self {
            Event::NoteOn {
                voice,
                key,
                velocity,
                tune_cents,
                ..
            } => Event::NoteOn {
                offset: at,
                voice,
                key,
                velocity,
                tune_cents,
            },
            Event::NoteOff { voice, key, .. } => Event::NoteOff {
                offset: at,
                voice,
                key,
            },
            Event::AllNotesOff { .. } => Event::AllNotesOff { offset: at },
            Event::ParamValue { id, value, .. } => Event::ParamValue {
                offset: at,
                id,
                value,
            },
        }
    }

    /// Rank of the event among events of the same offset: `ParamValue` (0), then every
    /// note event (1) in the order it was issued. A merge of sorted streams is by
    /// `(offset, rank)` and stable.
    pub const fn rank(&self) -> u8 {
        match self {
            Event::ParamValue { .. } => 0,
            Event::NoteOn { .. } | Event::NoteOff { .. } | Event::AllNotesOff { .. } => 1,
        }
    }

    /// A note event (the only events a MIDI effect's output forwards).
    pub const fn is_note(&self) -> bool {
        !matches!(self, Event::ParamValue { .. })
    }
}

/// Clamps a fine tuning to [`MAX_TUNE_CENTS`]; a non-finite value is 0.
pub fn clamp_tune_cents(cents: f32) -> f32 {
    if cents.is_finite() {
        cents.clamp(-MAX_TUNE_CENTS, MAX_TUNE_CENTS)
    } else {
        0.0
    }
}

/// Where a block's events stand in it: `offset` clamped to the last frame and never before
/// `cursor` (the frame already reached), so that a broken guarantee never panics.
pub fn event_frame(offset: u32, cursor: usize, frames: usize) -> usize {
    (offset as usize).min(frames.saturating_sub(1)).max(cursor)
}

// ---------------------------------------------------------------- the event output

/// A fixed-capacity, sorted buffer of events: the output of a MIDI effect. Built off the
/// audio path; [`EventBuffer::push`] never allocates.
///
/// The host sizes it in `prepare` so that it never fills: [`MidiOutput::capacity`] from
/// the descriptor's bounds and the most events the effect can receive in one block (the
/// host cuts a block into sub-blocks when the events of its tables fill a fixed room, so
/// the input is bounded whatever their size, MO-4; an effect after another is sized from
/// the output of the largest before it). Overflow is a safety net for an effect that
/// breaks its declared bounds: one slot is kept in reserve; when a push is refused, the
/// buffer counts it and [`EventBuffer::finish`] appends an [`Event::AllNotesOff`] in that
/// slot, after every event pushed, so that a lost note-off never leaves a note stuck.
#[derive(Debug)]
pub struct EventBuffer {
    events: Box<[Event]>,
    len: usize,
    overflowed: u32,
}

impl EventBuffer {
    /// A buffer of `capacity` events (at least 1: the reserved slot), or a typed error when
    /// the memory is refused.
    pub fn try_with_capacity(capacity: usize) -> Result<Self, MsgError> {
        let capacity = capacity.max(1);
        let mut events = Vec::new();
        events
            .try_reserve_exact(capacity)
            .map_err(|_| MsgError::OutOfMemory {
                bytes: capacity.saturating_mul(size_of::<Event>()),
            })?;
        events.resize(capacity, Event::AllNotesOff { offset: 0 });
        Ok(Self {
            events: events.into_boxed_slice(),
            len: 0,
            overflowed: 0,
        })
    }

    /// Slots, the reserved one included.
    pub fn capacity(&self) -> usize {
        self.events.len()
    }

    /// Empties the buffer and its overflow count, before a block.
    pub fn clear(&mut self) {
        self.len = 0;
        self.overflowed = 0;
    }

    /// Appends an event; `false` (and counted) when only the reserved slot is left. An
    /// offset earlier than the last event's is raised to it: the buffer stays sorted.
    pub fn push(&mut self, event: Event) -> bool {
        if self.len + 1 >= self.events.len() {
            self.overflowed = self.overflowed.saturating_add(1);
            return false;
        }
        let last = self.last_offset();
        self.events[self.len] = if event.offset() < last {
            event.at(last)
        } else {
            event
        };
        self.len += 1;
        true
    }

    /// Closes the block: after an overflow, appends `AllNotesOff` at the last offset in
    /// the reserved slot (once). The host calls it after the device's `process`.
    pub fn finish(&mut self) {
        // A push is refused only once the reserved slot is the last one free, so after an
        // overflow `len == capacity - 1` until this fills it; a second call finds it full.
        if self.overflowed > 0 && self.len < self.events.len() {
            self.events[self.len] = Event::AllNotesOff {
                offset: self.last_offset(),
            };
            self.len += 1;
        }
    }

    /// Keeps the note events only, in their order, in place (no allocation): the host
    /// forwards nothing else of a MIDI effect's output to the next device.
    pub fn retain_notes(&mut self) {
        let mut kept = 0;
        for i in 0..self.len {
            let event = self.events[i];
            if event.is_note() {
                self.events[kept] = event;
                kept += 1;
            }
        }
        self.len = kept;
    }

    /// Pushes refused since the last [`EventBuffer::clear`].
    pub fn overflowed(&self) -> u32 {
        self.overflowed
    }

    pub fn events(&self) -> &[Event] {
        &self.events[..self.len]
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn last_offset(&self) -> u32 {
        if self.len == 0 {
            0
        } else {
            self.events[self.len - 1].offset()
        }
    }
}

// ---------------------------------------------------------------- block context and io

/// The transport as a device sees it at the first frame of a block.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transport {
    pub playing: bool,
    pub tempo: Tempo,
    pub signature: TimeSignature,
    /// Position of the block's first frame, in ticks (fractional: 960 per quarter note).
    pub position_ticks: f64,
}

impl Default for Transport {
    fn default() -> Self {
        Self {
            playing: false,
            tempo: Tempo::default(),
            signature: TimeSignature::default(),
            position_ticks: 0.0,
        }
    }
}

/// The bounds a MIDI effect declares on its output, for the host to size the buffer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct MidiOutput {
    /// Most events written for one input event (1 for a transposer, the voices of a chord).
    pub per_input: u32,
    /// Most events written per block on top of those (an arpeggiator's own notes).
    pub per_block: u32,
}

impl MidiOutput {
    /// Capacity of the output buffer for `input_events` events in a block, the reserved
    /// slot included.
    pub fn capacity(&self, input_events: usize) -> usize {
        (self.per_input as usize)
            .saturating_mul(input_events)
            .saturating_add(self.per_block as usize)
            .saturating_add(1)
    }
}

/// What a device knows of the block it renders.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlockCtx {
    pub sample_rate: f32,
    /// Frames of the block: the length of both channels of [`AudioIo`].
    pub frames: u32,
    pub transport: Transport,
}

/// The buffers of one block.
///
/// - An instrument receives zeroed channels and writes its sound.
/// - An audio effect transforms the channels in place.
/// - A MIDI effect leaves the channels untouched and writes its notes to `events_out`. The
///   host forwards only the note events of that output to the next device (the
///   `ParamValue`s a MIDI effect writes are dropped), and ignores `events_out` for the
///   other roles.
#[derive(Debug)]
pub struct AudioIo<'a> {
    pub left: &'a mut [f32],
    pub right: &'a mut [f32],
    pub events_out: &'a mut EventBuffer,
}

/// What a device is built for, in `prepare`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DeviceSetup {
    pub sample_rate: f32,
    /// The largest block `process` will receive.
    pub max_block_frames: usize,
}

impl Default for DeviceSetup {
    fn default() -> Self {
        Self {
            sample_rate: 48_000.0,
            max_block_frames: limits::MAX_BLOCK_FRAMES,
        }
    }
}

// ---------------------------------------------------------------- the trait

/// The contract every device implements (module doc). Every method runs on the audio
/// thread: no allocation, no lock, no I/O, no panic on any input.
pub trait Device: Send {
    /// The static description of this device's kind.
    fn descriptor(&self) -> &'static DeviceDescriptor;

    /// Renders one block: applies `events` at their offsets, reads and writes `io`.
    fn process(&mut self, ctx: &BlockCtx, events: &[Event], io: &mut AudioIo<'_>);

    /// Sets a parameter as a [`Event::ParamValue`] at offset 0 of the next block would.
    fn set_param(&mut self, id: ParamId, value: f32);

    /// Clears the DSP state (voices, tails) and snaps smoothed parameters to their
    /// targets; parameter values are kept.
    fn reset(&mut self);

    /// Voices sounding after the last block, releasing ones included, for the visual
    /// feedback (MO-5); 0 for a device that has no voices.
    fn voices(&self) -> u32 {
        0
    }
}

/// Builds a device off the audio path, with fallible allocations only.
pub type Factory = fn(&DeviceSetup) -> Result<Box<dyn Device>, MsgError>;

// ---------------------------------------------------------------- descriptors

/// The unit of a parameter's value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize), serde(rename_all = "snake_case"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum Unit {
    /// A plain number (a level from 0 to 1, an index).
    None,
    /// A linear gain (an amplitude factor, 1 = unity): shown in decibels, `20 log10(value)`.
    Gain,
    Seconds,
    Hertz,
    Decibels,
    Semitones,
    Cents,
    Percent,
}

/// How a control maps its travel to the value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize), serde(rename_all = "snake_case"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum Scale {
    Linear,
    /// Logarithmic travel (frequencies, times); requires `min > 0`.
    Log,
}

/// One parameter of a device kind.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ParamDescriptor {
    /// Stable numeric id, the one events and `ControlMsg::ParamSet` carry.
    #[cfg_attr(feature = "schema", schemars(with = "u32"))]
    pub id: ParamId,
    /// Stable string id: the key of the project's parameter map and of the translated
    /// label and description in the UI.
    pub key: &'static str,
    pub unit: Unit,
    pub min: f64,
    pub max: f64,
    pub default: f64,
    pub scale: Scale,
    /// Smallest change of the value; 0 for a continuous parameter.
    pub step: f64,
    /// Time a new value takes to be reached (linear ramp), in seconds; 0 applies the value
    /// at its exact offset.
    pub smoothing_seconds: f64,
    /// For an enumerated parameter, the key of each value from `min` to `max` (translated
    /// by the UI); empty otherwise.
    pub values: &'static [&'static str],
}

impl ParamDescriptor {
    /// A value as the device applies it: `None` when not finite, else clamped to the range
    /// and rounded to the step.
    pub fn sanitize(&self, value: f32) -> Option<f32> {
        if !value.is_finite() {
            return None;
        }
        let mut v = f64::from(value).clamp(self.min, self.max);
        if self.step > 0.0 {
            v = (self.min + ((v - self.min) / self.step).round() * self.step).min(self.max);
        }
        Some(v as f32)
    }

    /// Frames a smoothed value takes to reach its target at `sample_rate`.
    pub fn smoothing_frames(&self, sample_rate: f32) -> u32 {
        let frames = (self.smoothing_seconds * f64::from(sample_rate)).round();
        if frames.is_finite() && frames > 0.0 {
            frames.min(f64::from(u32::MAX)) as u32
        } else {
            0
        }
    }
}

/// The static description of a device kind: exported in the schema bundle, so that the UI
/// generates the device's panel and agents know its parameters.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct DeviceDescriptor {
    /// Stable type id, `faulk.<name>`: `DeviceSource::Builtin::id` in the project.
    pub kind: &'static str,
    /// The number `ControlMsg::DeviceInsert` carries; never reused nor renumbered.
    #[cfg_attr(feature = "schema", schemars(with = "u32"))]
    pub kind_id: DeviceKind,
    /// Version of the kind (`DeviceSource::Builtin::version`), raised when its parameters
    /// change meaning.
    pub version: u32,
    /// The displayed name: one English name, never translated.
    pub name: &'static str,
    pub role: DeviceRole,
    /// Most memory an instance takes, in bytes, at [`MAX_SAMPLE_RATE`] and
    /// [`limits::MAX_BLOCK_FRAMES`].
    pub memory_bytes: u64,
    /// The bounds of a MIDI effect's output; `None` for the other roles.
    pub midi_output: Option<MidiOutput>,
    pub params: &'static [ParamDescriptor],
}

/// Why a descriptor is invalid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DescriptorError {
    /// `kind` is not `faulk.` followed by lowercase letters, digits or `_`.
    BadKind,
    /// `name` is empty, not ASCII, or has surrounding spaces.
    BadName,
    /// `kind_id` is 0 (reserved for "none").
    BadKindId,
    /// `midi_output` missing (or both bounds 0) on a MIDI effect, or set on another role.
    BadMidiOutput,
    /// More than [`MAX_PARAMS_PER_DEVICE`] parameters.
    TooManyParams,
    /// Two parameters share an id or a key.
    DuplicateParam(u32),
    /// A key is empty or not lowercase ASCII with `_`.
    BadKey(u32),
    /// A bound is not finite, or `min >= max`.
    BadRange(u32),
    /// `default` is out of range or not on the step.
    BadDefault(u32),
    /// A log scale with `min <= 0`.
    BadScale(u32),
    /// A negative step, or one that does not divide the range.
    BadStep(u32),
    /// A smoothing time out of `[0, MAX_SMOOTHING_SECONDS]`, or above 0 on a stepped
    /// parameter.
    BadSmoothing(u32),
    /// An enumeration whose values do not match `min..=max` with step 1.
    BadValues(u32),
}

/// A key: a lowercase ASCII letter, then lowercase letters, digits and `_`, at most
/// [`MAX_KEY_LEN`] bytes (a subset of the project's key rule).
fn is_key(s: &str) -> bool {
    s.len() <= MAX_KEY_LEN
        && s.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

fn is_multiple(span: f64, step: f64) -> bool {
    let n = span / step;
    (n - n.round()).abs() < 1e-9
}

impl DeviceDescriptor {
    /// Checks the rules every descriptor follows (tested on every registered one).
    pub fn validate(&self) -> Result<(), DescriptorError> {
        match self.kind.strip_prefix("faulk.") {
            Some(rest) if is_key(rest) && self.kind.len() <= MAX_KEY_LEN => {}
            _ => return Err(DescriptorError::BadKind),
        }
        if self.name.is_empty() || !self.name.is_ascii() || self.name.trim() != self.name {
            return Err(DescriptorError::BadName);
        }
        if self.kind_id.0 == 0 {
            return Err(DescriptorError::BadKindId);
        }
        let midi_ok = match (self.role, self.midi_output) {
            (DeviceRole::MidiEffect, Some(m)) => m.per_input > 0 || m.per_block > 0,
            (DeviceRole::MidiEffect, None) => false,
            (_, out) => out.is_none(),
        };
        if !midi_ok {
            return Err(DescriptorError::BadMidiOutput);
        }
        if self.params.len() > MAX_PARAMS_PER_DEVICE {
            return Err(DescriptorError::TooManyParams);
        }
        for (i, p) in self.params.iter().enumerate() {
            let id = p.id.0;
            if self.params[..i]
                .iter()
                .any(|q| q.id == p.id || q.key == p.key)
            {
                return Err(DescriptorError::DuplicateParam(id));
            }
            if !is_key(p.key) {
                return Err(DescriptorError::BadKey(id));
            }
            if !(p.min.is_finite() && p.max.is_finite() && p.min < p.max) {
                return Err(DescriptorError::BadRange(id));
            }
            if p.scale == Scale::Log && p.min <= 0.0 {
                return Err(DescriptorError::BadScale(id));
            }
            if !(p.step >= 0.0 && p.step.is_finite())
                || (p.step > 0.0 && !is_multiple(p.max - p.min, p.step))
            {
                return Err(DescriptorError::BadStep(id));
            }
            if !(p.default.is_finite()
                && (p.min..=p.max).contains(&p.default)
                && (p.step == 0.0 || is_multiple(p.default - p.min, p.step)))
            {
                return Err(DescriptorError::BadDefault(id));
            }
            if !(0.0..=MAX_SMOOTHING_SECONDS).contains(&p.smoothing_seconds)
                || (p.step > 0.0 && p.smoothing_seconds > 0.0)
            {
                return Err(DescriptorError::BadSmoothing(id));
            }
            if !p.values.is_empty()
                && (p.step != 1.0
                    || p.values.len() as f64 != p.max - p.min + 1.0
                    || !p.values.iter().all(|v| is_key(v)))
            {
                return Err(DescriptorError::BadValues(id));
            }
        }
        Ok(())
    }

    /// The parameter of this kind with `id`.
    pub fn param(&self, id: ParamId) -> Option<&'static ParamDescriptor> {
        self.params.iter().find(|p| p.id == id)
    }
}

/// The descriptors of Faulk's built-in devices, in the order of the registry
/// ([`crate::devices::REGISTRY`]). Exported in the schema bundle (`crates/protocol`).
pub fn builtin_descriptors() -> impl Iterator<Item = &'static DeviceDescriptor> + Clone {
    crate::devices::REGISTRY.iter().map(|b| b.descriptor)
}

/// The built-in descriptor of `kind_id`.
pub fn builtin(kind_id: DeviceKind) -> Option<&'static DeviceDescriptor> {
    crate::devices::builtin(kind_id).map(|b| b.descriptor)
}

#[cfg(feature = "testing")]
pub mod testing;
