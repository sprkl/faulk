//! Payload codecs of [`ControlMsg`] (MO-1): the byte layout of every variant after its
//! [`Header`], little-endian.
//!
//! Every field is written at its natural width, without padding inside the payload:
//!
//! | Type | Bytes |
//! |---|---|
//! | `bool` | `u8`, 0 or 1 (anything else is refused) |
//! | `Option<T>` | `u8` flag 0 or 1, then `T` **always** (zeros when absent, ignored on read) |
//! | `f32`, `f64` | IEEE bits; a non-finite value is refused |
//! | slots, ids, counts | `u32`, except [`DeviceSlot`] and [`SceneSlot`]: `u16` |
//! | [`Tick`] | `i64` |
//! | [`Frame`] | `u64` |
//! | [`Tempo`] | `f64` beats per minute, through [`Tempo::new`] |
//! | [`TimeSignature`] | numerator `u8`, denominator `u8`, through [`TimeSignature::new`] |
//! | [`Quantize`] | `u8`, its index in [`Quantize::ALL`] |
//! | [`LoopRegion`] | start `i64`, end `i64`, through [`LoopRegion::new`]; a clip's loop (`ClipNotesUpload`) must lie within `[0, length]` |
//! | [`TrackKind`], [`Curve`] | `u8` discriminant |
//! | [`AutomationTarget`] | kind `u8` (0 device parameter, 1 mixer gain, 2 mixer pan), device `u16`, param `u32` (7 bytes) |
//! | [`Span`] | kind `u8` (0 ticks, 1 frames), offset and length as 64-bit values (17 bytes) |
//! | [`Note`] | start `i64`, length `i64`, pitch `u8` ≤ 127, velocity `u8` ≤ 127, cents `i8` in ±50, chance `u8` ≤ 100, repeat `u8` in 1..=16 (21 bytes) |
//! | [`AutomationPoint`] | tick `i64`, value `f32`, curve `u8` (13 bytes) |
//! | [`Placement`] | placement `u32`, clip `u32`, start `i64`, span (33 bytes) |
//! | a list | count `u32`, then the items; the count must match the bytes left exactly |
//!
//! The fields of each variant follow in the order of their declaration in [`ControlMsg`];
//! [`ControlMsg::Reset`] has an empty payload (its epoch is the header's), and the `state`
//! of [`ControlMsg::DeviceInsert`] is the rest of its payload: `(param id u32, value f32)`
//! pairs, 8 bytes each (the initial values of the device's parameters, CT-5).
//!
//! Decoding reads **exactly** the header's `len` bytes: a payload too short, too long, or
//! with a value out of its range is [`MsgError::BadPayload`] (the message is skipped by its
//! length and counted, the stream goes on). Every read is bounds-checked, a list's count is
//! checked against the bytes left before anything is allocated, and the allocation is
//! fallible ([`MsgError::OutOfMemory`]): no input makes the decoder panic or abort.

use super::{
    AutomationPoint, AutomationTarget, ClipSlot, ControlMsg, Curve, DeviceKind, DeviceSlot,
    HEADER_BYTES, Header, LaneSlot, MSG_ALIGN, MSG_VERSION, MsgError, Note, ParamId, Placement,
    PlacementSlot, SceneSlot, SourceSlot, Span, Tag, TrackKind, TrackSlot, limits,
};
use crate::time::{Frame, LoopRegion, Quantize, Tempo, Tick, TimeSignature};

/// Bytes of a [`Note`] on the wire.
pub const NOTE_WIRE_BYTES: usize = 21;
/// Bytes of an [`AutomationPoint`] on the wire.
pub const POINT_WIRE_BYTES: usize = 13;
/// Bytes of a [`Placement`] on the wire.
pub const PLACEMENT_WIRE_BYTES: usize = 33;
/// Bytes of one `(param id, value)` pair of a device's initial state.
pub const STATE_PAIR_BYTES: usize = 8;

/// Highest MIDI value (pitch, velocity).
const MIDI_MAX: u8 = 127;

// ---------------------------------------------------------------- writer

struct Writer<'a>(&'a mut Vec<u8>);

impl Writer<'_> {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn i64(&mut self, v: i64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn f32(&mut self, v: f32) {
        self.u32(v.to_bits());
    }
    fn f64(&mut self, v: f64) {
        self.u64(v.to_bits());
    }
    fn bool(&mut self, v: bool) {
        self.u8(u8::from(v));
    }
    fn count(&mut self, n: usize) {
        // A list longer than u32::MAX cannot fit under MAX_CONTROL_MSG_BYTES anyway: the
        // encoder refuses the message by its length before it is sent.
        self.u32(u32::try_from(n).unwrap_or(u32::MAX));
    }
    fn note(&mut self, n: &Note) {
        self.i64(n.start.0);
        self.i64(n.length.0);
        self.u8(n.pitch);
        self.u8(n.velocity);
        self.u8(n.cents as u8);
        self.u8(n.chance);
        self.u8(n.repeat);
    }
    fn point(&mut self, p: &AutomationPoint) {
        self.i64(p.tick.0);
        self.f32(p.value);
        self.u8(p.curve as u8);
    }
    fn span(&mut self, s: &Span) {
        match *s {
            Span::Ticks { offset, length } => {
                self.u8(0);
                self.i64(offset.0);
                self.i64(length.0);
            }
            Span::Frames { offset, length } => {
                self.u8(1);
                self.u64(offset.0);
                self.u64(length.0);
            }
        }
    }
    fn placement(&mut self, p: &Placement) {
        self.u32(p.placement.0);
        self.u32(p.clip.0);
        self.i64(p.start.0);
        self.span(&p.span);
    }
    fn target(&mut self, t: &AutomationTarget) {
        let (kind, device, param) = match *t {
            AutomationTarget::DeviceParam { device, param } => (0, device.0, param.0),
            AutomationTarget::MixerGain => (1, 0, 0),
            AutomationTarget::MixerPan => (2, 0, 0),
        };
        self.u8(kind);
        self.u16(device);
        self.u32(param);
    }
}

// ---------------------------------------------------------------- reader

/// A malformed payload (the caller turns it into [`MsgError::BadPayload`]).
#[derive(Debug)]
pub(super) enum Bad {
    Payload,
    OutOfMemory(usize),
}

impl From<core::array::TryFromSliceError> for Bad {
    fn from(_: core::array::TryFromSliceError) -> Self {
        Bad::Payload
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
}

fn check(ok: bool) -> Result<(), Bad> {
    if ok { Ok(()) } else { Err(Bad::Payload) }
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], Bad> {
        let (head, rest) = self.bytes.split_at_checked(n).ok_or(Bad::Payload)?;
        self.bytes = rest;
        Ok(head)
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N], Bad> {
        Ok(self.take(N)?.try_into()?)
    }
    fn u8(&mut self) -> Result<u8, Bad> {
        Ok(self.array::<1>()?[0])
    }
    fn u16(&mut self) -> Result<u16, Bad> {
        Ok(u16::from_le_bytes(self.array()?))
    }
    fn u32(&mut self) -> Result<u32, Bad> {
        Ok(u32::from_le_bytes(self.array()?))
    }
    fn i64(&mut self) -> Result<i64, Bad> {
        Ok(i64::from_le_bytes(self.array()?))
    }
    fn u64(&mut self) -> Result<u64, Bad> {
        Ok(u64::from_le_bytes(self.array()?))
    }
    fn f32(&mut self) -> Result<f32, Bad> {
        let v = f32::from_bits(self.u32()?);
        check(v.is_finite())?;
        Ok(v)
    }
    fn f64(&mut self) -> Result<f64, Bad> {
        let v = f64::from_bits(self.u64()?);
        check(v.is_finite())?;
        Ok(v)
    }
    fn bool(&mut self) -> Result<bool, Bad> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(Bad::Payload),
        }
    }
    fn midi(&mut self) -> Result<u8, Bad> {
        let v = self.u8()?;
        check(v <= MIDI_MAX)?;
        Ok(v)
    }
    fn track(&mut self) -> Result<TrackSlot, Bad> {
        Ok(TrackSlot(self.u32()?))
    }
    fn device(&mut self) -> Result<DeviceSlot, Bad> {
        Ok(DeviceSlot(self.u16()?))
    }
    fn clip(&mut self) -> Result<ClipSlot, Bad> {
        Ok(ClipSlot(self.u32()?))
    }
    fn scene(&mut self) -> Result<SceneSlot, Bad> {
        Ok(SceneSlot(self.u16()?))
    }
    fn tick(&mut self) -> Result<Tick, Bad> {
        Ok(Tick(self.i64()?))
    }
    fn frame(&mut self) -> Result<Frame, Bad> {
        Ok(Frame(self.u64()?))
    }
    /// An `Option`: the flag, then the value always read.
    fn option<T>(
        &mut self,
        read: impl FnOnce(&mut Self) -> Result<T, Bad>,
    ) -> Result<Option<T>, Bad> {
        let present = self.bool()?;
        let value = read(self)?;
        Ok(present.then_some(value))
    }
    fn note(&mut self) -> Result<Note, Bad> {
        Ok(Note {
            start: self.tick()?,
            length: self.tick()?,
            pitch: self.midi()?,
            velocity: self.midi()?,
            cents: {
                let v = self.u8()? as i8;
                check(v.unsigned_abs() <= Note::MAX_CENTS.unsigned_abs())?;
                v
            },
            chance: {
                let v = self.u8()?;
                check(v <= Note::MAX_CHANCE)?;
                v
            },
            repeat: {
                let v = self.u8()?;
                check((1..=Note::MAX_REPEAT).contains(&v))?;
                v
            },
        })
    }
    fn curve(&mut self) -> Result<Curve, Bad> {
        match self.u8()? {
            0 => Ok(Curve::Step),
            1 => Ok(Curve::Linear),
            _ => Err(Bad::Payload),
        }
    }
    fn point(&mut self) -> Result<AutomationPoint, Bad> {
        Ok(AutomationPoint {
            tick: self.tick()?,
            value: self.f32()?,
            curve: self.curve()?,
        })
    }
    fn span(&mut self) -> Result<Span, Bad> {
        let kind = self.u8()?;
        let (a, b) = (self.array::<8>()?, self.array::<8>()?);
        match kind {
            0 => Ok(Span::Ticks {
                offset: Tick(i64::from_le_bytes(a)),
                length: Tick(i64::from_le_bytes(b)),
            }),
            1 => Ok(Span::Frames {
                offset: Frame(u64::from_le_bytes(a)),
                length: Frame(u64::from_le_bytes(b)),
            }),
            _ => Err(Bad::Payload),
        }
    }
    fn placement(&mut self) -> Result<Placement, Bad> {
        Ok(Placement {
            placement: PlacementSlot(self.u32()?),
            clip: self.clip()?,
            start: self.tick()?,
            span: self.span()?,
        })
    }
    fn target(&mut self) -> Result<AutomationTarget, Bad> {
        let kind = self.u8()?;
        let device = self.device()?;
        let param = ParamId(self.u32()?);
        match kind {
            0 => Ok(AutomationTarget::DeviceParam { device, param }),
            1 => Ok(AutomationTarget::MixerGain),
            2 => Ok(AutomationTarget::MixerPan),
            _ => Err(Bad::Payload),
        }
    }
    fn track_kind(&mut self) -> Result<TrackKind, Bad> {
        match self.u8()? {
            0 => Ok(TrackKind::Midi),
            1 => Ok(TrackKind::Audio),
            _ => Err(Bad::Payload),
        }
    }
    /// A list: the count must match the bytes left exactly (checked before allocating),
    /// then a fallible allocation of exactly that many items.
    fn list<T>(
        &mut self,
        item_bytes: usize,
        read: impl Fn(&mut Self) -> Result<T, Bad>,
    ) -> Result<Vec<T>, Bad> {
        let count = self.u32()? as usize;
        check(count.checked_mul(item_bytes) == Some(self.bytes.len()))?;
        let mut items = Vec::new();
        items
            .try_reserve_exact(count)
            .map_err(|_| Bad::OutOfMemory(count.saturating_mul(size_of::<T>())))?;
        for _ in 0..count {
            items.push(read(self)?);
        }
        Ok(items)
    }
    /// The rest of the payload, copied through a fallible allocation.
    fn rest(&mut self) -> Result<Vec<u8>, Bad> {
        let rest = self.take(self.bytes.len())?;
        let mut out = Vec::new();
        out.try_reserve_exact(rest.len())
            .map_err(|_| Bad::OutOfMemory(rest.len()))?;
        out.extend_from_slice(rest);
        Ok(out)
    }
    fn end(&self) -> Result<(), Bad> {
        check(self.bytes.is_empty())
    }
}

// ---------------------------------------------------------------- ControlMsg

impl ControlMsg {
    /// Appends this message to `out`: its [`Header`] (with `epoch`, except a
    /// [`ControlMsg::Reset`], whose header carries its own epoch, and `seq`), its payload,
    /// and zeros up to the next [`MSG_ALIGN`] boundary. A payload over
    /// [`limits::MAX_CONTROL_MSG_BYTES`] is [`MsgError::TooLarge`] and nothing is appended:
    /// the sender splits it into chunks. A value the decoder would refuse is
    /// [`MsgError::BadPayload`], and nothing is appended either.
    pub fn encode(&self, epoch: u16, seq: u32, out: &mut Vec<u8>) -> Result<(), MsgError> {
        let start = out.len();
        out.extend_from_slice(&[0; HEADER_BYTES]);
        self.write_payload(&mut Writer(out));
        let len = out.len() - start - HEADER_BYTES;
        let len = match u32::try_from(len) {
            Ok(len) if len <= limits::MAX_CONTROL_MSG_BYTES => len,
            _ => {
                out.truncate(start);
                return Err(MsgError::TooLarge(u32::try_from(len).unwrap_or(u32::MAX)));
            }
        };
        let epoch = match self {
            ControlMsg::Reset { epoch } => *epoch,
            _ => epoch,
        };
        let header = Header {
            version: MSG_VERSION,
            tag: self.tag(),
            epoch,
            len,
            seq,
        };
        // What the decoder would refuse (a non-finite float, a MIDI value above 127, a
        // device state that is not whole pairs) is refused here, where the mistake is made.
        if let Err(error) = ControlMsg::decode(&header, &out[start + HEADER_BYTES..]) {
            out.truncate(start);
            return Err(error);
        }
        out[start..start + HEADER_BYTES].copy_from_slice(&header.encode());
        out.resize(start + Header::frame_len(len), 0);
        Ok(())
    }

    fn write_payload(&self, w: &mut Writer<'_>) {
        match self {
            ControlMsg::Reset { .. }
            | ControlMsg::StopAll
            | ControlMsg::AllSoundOff
            | ControlMsg::BackToArrangement
            | ControlMsg::Stop => {}
            ControlMsg::SetTransport {
                tempo,
                signature,
                launch_quantize,
            } => {
                w.f64(tempo.bpm());
                w.u8(signature.numerator());
                w.u8(signature.denominator());
                let index = Quantize::ALL
                    .iter()
                    .position(|q| q == launch_quantize)
                    .unwrap_or(0);
                w.u8(index as u8);
            }
            ControlMsg::SetMetronome {
                enabled,
                volume,
                count_in_bars,
            } => {
                w.bool(*enabled);
                w.f32(*volume);
                w.u8(*count_in_bars);
            }
            ControlMsg::SetMaster { gain, pan, mute } => {
                w.f32(*gain);
                w.f32(*pan);
                w.bool(*mute);
            }
            ControlMsg::TrackCreate { track, kind } => {
                w.u32(track.0);
                w.u8(*kind as u8);
            }
            ControlMsg::TrackRemove { track }
            | ControlMsg::StopTrack { track }
            | ControlMsg::BackTrackToArrangement { track }
            | ControlMsg::AllNotesOff { track } => w.u32(track.0),
            ControlMsg::TrackSetMix {
                track,
                gain,
                pan,
                mute,
            } => {
                w.u32(track.0);
                w.f32(*gain);
                w.f32(*pan);
                w.bool(*mute);
            }
            ControlMsg::TrackSetSolo { track, solo: on }
            | ControlMsg::TrackArm { track, armed: on }
            | ControlMsg::TrackSetMonitor { track, monitor: on } => {
                w.u32(track.0);
                w.bool(*on);
            }
            ControlMsg::SelectTrack { track } => {
                w.bool(track.is_some());
                w.u32(track.map_or(0, |t| t.0));
            }
            ControlMsg::DeviceInsert {
                track,
                device,
                kind,
                state,
            } => {
                w.u32(track.0);
                w.u16(device.0);
                w.u32(kind.0);
                w.0.extend_from_slice(state);
            }
            ControlMsg::DeviceRemove { track, device } => {
                w.u32(track.0);
                w.u16(device.0);
            }
            ControlMsg::DeviceSetEnabled {
                track,
                device,
                enabled,
            } => {
                w.u32(track.0);
                w.u16(device.0);
                w.bool(*enabled);
            }
            ControlMsg::ParamSet {
                track,
                device,
                param,
                value,
            } => {
                w.u32(track.0);
                w.u16(device.0);
                w.u32(param.0);
                w.f32(*value);
            }
            ControlMsg::ClipNotesUpload {
                clip,
                length,
                looping,
                first,
                total,
                notes,
            } => {
                w.u32(clip.0);
                w.i64(length.0);
                w.bool(looping.is_some());
                w.i64(looping.map_or(0, |r| r.start().0));
                w.i64(looping.map_or(0, |r| r.end().0));
                w.u32(*first);
                w.u32(*total);
                w.count(notes.len());
                notes.iter().for_each(|n| w.note(n));
            }
            ControlMsg::ClipRemove { clip } => w.u32(clip.0),
            ControlMsg::AudioSourceDeclare {
                source,
                sample_rate,
                channels,
                frames,
            } => {
                w.u32(source.0);
                w.u32(*sample_rate);
                w.u8(*channels);
                w.u64(frames.0);
            }
            ControlMsg::AudioSourceRelease { source } => w.u32(source.0),
            ControlMsg::ClipAudioSet {
                clip,
                source,
                start,
                length,
            } => {
                w.u32(clip.0);
                w.u32(source.0);
                w.u64(start.0);
                w.u64(length.0);
            }
            ControlMsg::TrackPlacementsUpload {
                track,
                first,
                total,
                placements,
            } => {
                w.u32(track.0);
                w.u32(*first);
                w.u32(*total);
                w.count(placements.len());
                placements.iter().for_each(|p| w.placement(p));
            }
            ControlMsg::AutomationLaneUpload {
                lane,
                track,
                target,
                first,
                total,
                points,
            } => {
                w.u32(lane.0);
                w.u32(track.0);
                w.target(target);
                w.u32(*first);
                w.u32(*total);
                w.count(points.len());
                points.iter().for_each(|p| w.point(p));
            }
            ControlMsg::AutomationLaneRemove { lane } => w.u32(lane.0),
            ControlMsg::SceneSetSlot { track, scene, clip } => {
                w.u32(track.0);
                w.u16(scene.0);
                w.bool(clip.is_some());
                w.u32(clip.map_or(0, |c| c.0));
            }
            ControlMsg::LaunchClip { track, scene } => {
                w.u32(track.0);
                w.u16(scene.0);
            }
            ControlMsg::LaunchScene { scene } => w.u16(scene.0),
            ControlMsg::Play { from: tick } | ControlMsg::Seek { tick } => w.i64(tick.0),
            ControlMsg::SetLoop { region } => {
                w.bool(region.is_some());
                w.i64(region.map_or(0, |r| r.start().0));
                w.i64(region.map_or(0, |r| r.end().0));
            }
            ControlMsg::SetRecording { recording } => w.bool(*recording),
            ControlMsg::NoteOn {
                track,
                pitch,
                velocity,
            } => {
                w.u32(track.0);
                w.u8(*pitch);
                w.u8(*velocity);
            }
            ControlMsg::NoteOff { track, pitch } => {
                w.u32(track.0);
                w.u8(*pitch);
            }
        }
    }

    /// Decodes the payload of the message `header` announces (`payload` is exactly its
    /// `len` bytes, as [`Header::decode`] checked they exist). Never panics: a malformed
    /// payload is [`MsgError::BadPayload`], a list too large for the memory
    /// [`MsgError::OutOfMemory`].
    pub fn decode(header: &Header, payload: &[u8]) -> Result<ControlMsg, MsgError> {
        let mut r = Reader { bytes: payload };
        match Self::read(header, &mut r).and_then(|msg| r.end().map(|()| msg)) {
            Ok(msg) => Ok(msg),
            Err(Bad::Payload) => Err(MsgError::BadPayload {
                tag: header.tag as u16,
            }),
            Err(Bad::OutOfMemory(bytes)) => Err(MsgError::OutOfMemory { bytes }),
        }
    }

    fn read(header: &Header, r: &mut Reader<'_>) -> Result<ControlMsg, Bad> {
        Ok(match header.tag {
            Tag::Reset => ControlMsg::Reset {
                epoch: header.epoch,
            },
            Tag::SetTransport => {
                let tempo = Tempo::new(r.f64()?).ok_or(Bad::Payload)?;
                let (num, den) = (r.u8()?, r.u8()?);
                let signature = TimeSignature::new(num, den).ok_or(Bad::Payload)?;
                let launch_quantize = *Quantize::ALL.get(r.u8()? as usize).ok_or(Bad::Payload)?;
                ControlMsg::SetTransport {
                    tempo,
                    signature,
                    launch_quantize,
                }
            }
            Tag::SetMetronome => ControlMsg::SetMetronome {
                enabled: r.bool()?,
                volume: r.f32()?,
                count_in_bars: r.u8()?,
            },
            Tag::SetMaster => ControlMsg::SetMaster {
                gain: r.f32()?,
                pan: r.f32()?,
                mute: r.bool()?,
            },
            Tag::TrackCreate => ControlMsg::TrackCreate {
                track: r.track()?,
                kind: r.track_kind()?,
            },
            Tag::TrackRemove => ControlMsg::TrackRemove { track: r.track()? },
            Tag::TrackSetMix => ControlMsg::TrackSetMix {
                track: r.track()?,
                gain: r.f32()?,
                pan: r.f32()?,
                mute: r.bool()?,
            },
            Tag::TrackSetSolo => ControlMsg::TrackSetSolo {
                track: r.track()?,
                solo: r.bool()?,
            },
            Tag::TrackArm => ControlMsg::TrackArm {
                track: r.track()?,
                armed: r.bool()?,
            },
            Tag::TrackSetMonitor => ControlMsg::TrackSetMonitor {
                track: r.track()?,
                monitor: r.bool()?,
            },
            Tag::SelectTrack => ControlMsg::SelectTrack {
                track: r.option(Reader::track)?,
            },
            Tag::DeviceInsert => {
                let track = r.track()?;
                let device = r.device()?;
                let kind = DeviceKind(r.u32()?);
                check(r.bytes.len().is_multiple_of(STATE_PAIR_BYTES))?;
                // Every value finite, like any other float of a payload.
                check(state_pairs(r.bytes).all(|(_, value)| value.is_finite()))?;
                ControlMsg::DeviceInsert {
                    track,
                    device,
                    kind,
                    state: r.rest()?,
                }
            }
            Tag::DeviceRemove => ControlMsg::DeviceRemove {
                track: r.track()?,
                device: r.device()?,
            },
            Tag::DeviceSetEnabled => ControlMsg::DeviceSetEnabled {
                track: r.track()?,
                device: r.device()?,
                enabled: r.bool()?,
            },
            Tag::ParamSet => ControlMsg::ParamSet {
                track: r.track()?,
                device: r.device()?,
                param: ParamId(r.u32()?),
                value: r.f32()?,
            },
            Tag::ClipNotesUpload => {
                let clip = r.clip()?;
                let length = r.tick()?;
                let present = r.bool()?;
                let (start, end) = (r.tick()?, r.tick()?);
                let looping = if present {
                    // Within the clip: `[start, end)` inside `[0, length]`.
                    check(start.0 >= 0 && end <= length)?;
                    Some(LoopRegion::new(start, end).ok_or(Bad::Payload)?)
                } else {
                    None
                };
                ControlMsg::ClipNotesUpload {
                    clip,
                    length,
                    looping,
                    first: r.u32()?,
                    total: r.u32()?,
                    notes: r.list(NOTE_WIRE_BYTES, Reader::note)?,
                }
            }
            Tag::ClipRemove => ControlMsg::ClipRemove { clip: r.clip()? },
            Tag::AudioSourceDeclare => ControlMsg::AudioSourceDeclare {
                source: SourceSlot(r.u32()?),
                sample_rate: r.u32()?,
                channels: r.u8()?,
                frames: r.frame()?,
            },
            Tag::AudioSourceRelease => ControlMsg::AudioSourceRelease {
                source: SourceSlot(r.u32()?),
            },
            Tag::ClipAudioSet => ControlMsg::ClipAudioSet {
                clip: r.clip()?,
                source: SourceSlot(r.u32()?),
                start: r.frame()?,
                length: r.frame()?,
            },
            Tag::TrackPlacementsUpload => ControlMsg::TrackPlacementsUpload {
                track: r.track()?,
                first: r.u32()?,
                total: r.u32()?,
                placements: r.list(PLACEMENT_WIRE_BYTES, Reader::placement)?,
            },
            Tag::AutomationLaneUpload => ControlMsg::AutomationLaneUpload {
                lane: LaneSlot(r.u32()?),
                track: r.track()?,
                target: r.target()?,
                first: r.u32()?,
                total: r.u32()?,
                points: r.list(POINT_WIRE_BYTES, Reader::point)?,
            },
            Tag::AutomationLaneRemove => ControlMsg::AutomationLaneRemove {
                lane: LaneSlot(r.u32()?),
            },
            Tag::SceneSetSlot => ControlMsg::SceneSetSlot {
                track: r.track()?,
                scene: r.scene()?,
                clip: r.option(Reader::clip)?,
            },
            Tag::LaunchClip => ControlMsg::LaunchClip {
                track: r.track()?,
                scene: r.scene()?,
            },
            Tag::LaunchScene => ControlMsg::LaunchScene { scene: r.scene()? },
            Tag::StopTrack => ControlMsg::StopTrack { track: r.track()? },
            Tag::StopAll => ControlMsg::StopAll,
            Tag::BackToArrangement => ControlMsg::BackToArrangement,
            Tag::BackTrackToArrangement => ControlMsg::BackTrackToArrangement { track: r.track()? },
            Tag::Play => ControlMsg::Play { from: r.tick()? },
            Tag::Stop => ControlMsg::Stop,
            Tag::Seek => ControlMsg::Seek { tick: r.tick()? },
            Tag::SetLoop => {
                let present = r.bool()?;
                let (start, end) = (r.tick()?, r.tick()?);
                let region = if present {
                    Some(LoopRegion::new(start, end).ok_or(Bad::Payload)?)
                } else {
                    None
                };
                ControlMsg::SetLoop { region }
            }
            Tag::SetRecording => ControlMsg::SetRecording {
                recording: r.bool()?,
            },
            Tag::NoteOn => ControlMsg::NoteOn {
                track: r.track()?,
                pitch: r.midi()?,
                velocity: r.midi()?,
            },
            Tag::NoteOff => ControlMsg::NoteOff {
                track: r.track()?,
                pitch: r.midi()?,
            },
            Tag::AllNotesOff => ControlMsg::AllNotesOff { track: r.track()? },
            Tag::AllSoundOff => ControlMsg::AllSoundOff,
        })
    }
}

/// The `(param id, value)` pairs of a [`ControlMsg::DeviceInsert`] state, in order; a
/// trailing partial pair (never produced by the encoder, refused by the decoder) is ignored.
pub fn state_pairs(state: &[u8]) -> impl Iterator<Item = (ParamId, f32)> + '_ {
    state.as_chunks::<STATE_PAIR_BYTES>().0.iter().map(|pair| {
        let id = u32::from_le_bytes([pair[0], pair[1], pair[2], pair[3]]);
        let value = f32::from_le_bytes([pair[4], pair[5], pair[6], pair[7]]);
        (ParamId(id), value)
    })
}

/// Encodes the initial state of a device: its `(param id, value)` pairs.
pub fn encode_state(values: &[(ParamId, f32)]) -> Vec<u8> {
    let mut out = Vec::with_capacity(values.len() * STATE_PAIR_BYTES);
    for (id, value) in values {
        out.extend_from_slice(&id.0.to_le_bytes());
        out.extend_from_slice(&value.to_le_bytes());
    }
    out
}

const _: () = assert!(MSG_ALIGN.is_power_of_two());

/// Encodes `msgs` back to back with `epoch` and sequence numbers from `first_seq` (the
/// sender's batching, for tests and tools; the command layer writes its own stream).
pub fn encode_stream(epoch: u16, first_seq: u32, msgs: &[ControlMsg]) -> Result<Vec<u8>, MsgError> {
    let mut out = Vec::new();
    for (i, msg) in msgs.iter().enumerate() {
        msg.encode(epoch, first_seq.wrapping_add(i as u32), &mut out)?;
    }
    Ok(out)
}
