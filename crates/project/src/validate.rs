//! The invariants of a project document, checked by [`Project::validate`].
//!
//! A project changed only by commands stays valid (each command keeps it so, CT-4); a
//! project read from text (a file edited by hand, a line merge by git) is validated before
//! use, and every violation is reported with where it is, so that a merge's semantic
//! conflicts can be listed rather than only the first one. Two invariants hold by
//! construction and are refused on read instead: ids are unique within a map, and a lane
//! has at most one point per tick (maps refuse duplicate keys, `serde_util::unique_map`).

use crate::ids::Id;
use crate::limits::*;
use crate::model::*;
use engine::time::{Frame, Tick};
use std::collections::{BTreeMap, HashSet};
use std::fmt;

/// One broken invariant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Violation {
    /// Where: `tracks/<id>/clips/<id>`, `transport/loop_region`…
    pub path: String,
    pub kind: ViolationKind,
}

/// The kinds of broken invariants.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ViolationKind {
    /// A format version this crate does not read without migration.
    Format(u32),
    /// An empty project name.
    EmptyName,
    /// A name with leading or trailing white space.
    UntrimmedName,
    /// A name longer than [`MAX_NAME_CHARS`].
    NameTooLong,
    /// An id used twice in the project (two entities, wherever they are).
    DuplicateId(Id),
    /// A clip's slot names a scene that does not exist.
    UnknownScene,
    /// Two clips of a track in the same launch-grid slot.
    SlotTaken,
    /// A placement names a clip that is not in its track.
    UnknownClip,
    /// A lane names a device that is not in its track.
    UnknownDevice,
    /// A clip whose content does not match its track (audio clip in a MIDI track, or the
    /// reverse), or a placement whose span does not match its clip (ticks for MIDI,
    /// frames for audio).
    KindMismatch,
    /// A parameter id or built-in device id that is not well formed (a module's name is a
    /// display name, checked like the other names).
    BadKey,
    /// A note outside `[0, clip length]`.
    NoteOutsideClip,
    /// A note with a pitch above 127, a velocity outside `1..=127`, a length ≤ 0, cents
    /// outside `-50..=50`, a chance above 100 or a repeat outside `1..=16`.
    BadNote,
    /// A length that is zero, negative or above the safety limit.
    BadLength,
    /// A range past its source: an audio range past the file, an offset past the clip.
    BadRange,
    /// A loop that is empty or outside its clip.
    BadLoop,
    /// The transport loops without a loop region.
    LoopWithoutRegion,
    /// A position or an end outside `[0, MAX_TICK]` or `[0, MAX_FRAME]`.
    BadPosition,
    /// The tempo map is not a single point at tick 0 (constant tempo in the MVP).
    TempoMap,
    /// The meter map is not a single point at tick 0 (one time signature in the MVP).
    MeterMap,
    /// NaN or an infinity where a finite number is required (a gain may be `-inf`).
    NotFinite,
    /// A track, master or metronome gain, or a gain automation point, finite but outside
    /// [`MIN_GAIN_DB`], [`MAX_GAIN_DB`] (silence is `null` on a gain, never a value below it).
    GainOutOfRange,
    /// A pan outside `[-1, 1]`, on a mixer or on a pan automation point.
    PanOutOfRange,
    /// More than one instrument in a chain.
    TooManyInstruments,
    /// An instrument that is not first in its chain.
    InstrumentNotFirst,
    /// The clip player on a MIDI track or as an effect, or another instrument on an audio
    /// track.
    WrongInstrument,
    /// A device of the master's chain that is not an effect: the master holds effects only.
    NotAnEffect,
    /// An audio file with no channel or more than [`MAX_CHANNELS`].
    BadChannels,
    /// More count-in bars than [`MAX_COUNT_IN_BARS`].
    CountIn,
    /// An audio track without the clip player (a MIDI track may have an empty chain until
    /// an instrument is chosen).
    MissingInstrument,
    /// An audio file's sample rate outside [`SAMPLE_RATES`].
    BadSampleRate,
    /// More elements than a safety limit allows.
    TooMany { limit: usize },
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {:?}", self.path, self.kind)
    }
}

struct Checker<'p> {
    limits: Limits,
    out: Vec<Violation>,
    seen: HashSet<&'p Id>,
}

impl<'p> Checker<'p> {
    fn push(&mut self, path: &str, kind: ViolationKind) {
        self.out.push(Violation {
            path: path.to_owned(),
            kind,
        });
    }

    fn check(&mut self, ok: bool, path: &str, kind: ViolationKind) {
        if !ok {
            self.push(path, kind);
        }
    }

    fn id(&mut self, id: &'p Id, path: &str) {
        if !self.seen.insert(id) {
            self.push(path, ViolationKind::DuplicateId(id.clone()));
        }
    }

    fn name(&mut self, name: &str, path: &str, required: bool) {
        if required && name.is_empty() {
            self.push(path, ViolationKind::EmptyName);
        }
        self.check(name.trim() == name, path, ViolationKind::UntrimmedName);
        self.check(
            name.chars().count() <= MAX_NAME_CHARS,
            path,
            ViolationKind::NameTooLong,
        );
    }

    fn count(&mut self, n: usize, limit: usize, path: &str) {
        self.check(n <= limit, path, ViolationKind::TooMany { limit });
    }

    fn tick(&mut self, t: Tick, path: &str) {
        self.check(
            (0..=MAX_TICK).contains(&t.get()),
            path,
            ViolationKind::BadPosition,
        );
    }

    fn gain(&mut self, g: f64, path: &str) {
        self.check(
            g.is_finite() || g == f64::NEG_INFINITY,
            path,
            ViolationKind::NotFinite,
        );
    }

    /// A track, master or metronome gain: silence (`-inf`) or a finite level from
    /// [`MIN_GAIN_DB`] to [`MAX_GAIN_DB`]. Other values are refused, never clamped.
    fn bounded_gain(&mut self, g: f64, path: &str) {
        if g == f64::NEG_INFINITY {
            return;
        }
        if g.is_finite() {
            self.check(
                (MIN_GAIN_DB..=MAX_GAIN_DB).contains(&g),
                path,
                ViolationKind::GainOutOfRange,
            );
        } else {
            self.push(path, ViolationKind::NotFinite);
        }
    }

    fn mixer(&mut self, m: &Mixer, path: &str) {
        self.bounded_gain(m.gain_db, path);
        if m.pan.is_finite() {
            self.check(
                (-1.0..=1.0).contains(&m.pan),
                path,
                ViolationKind::PanOutOfRange,
            );
        } else {
            self.push(path, ViolationKind::NotFinite);
        }
    }
}

/// A parameter id or built-in device id: ASCII lower case, digits, `_` and `.`, starting
/// with a letter, at most [`MAX_KEY_LEN`] bytes.
fn is_key(s: &str) -> bool {
    s.len() <= MAX_KEY_LEN
        && s.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'.')
}

fn frame_ok(f: Frame) -> bool {
    f.get() <= MAX_FRAME
}

impl Project {
    /// Checks every invariant of the document and returns all the violations found.
    pub fn validate(&self) -> Result<(), Vec<Violation>> {
        self.validate_with(&Limits::SAFETY)
    }

    /// [`Project::validate`] with other count limits (tests, load bench).
    pub fn validate_with(&self, limits: &Limits) -> Result<(), Vec<Violation>> {
        let mut c = Checker {
            limits: *limits,
            out: Vec::new(),
            seen: HashSet::new(),
        };
        c.check(
            self.format == FORMAT_VERSION,
            "format",
            ViolationKind::Format(self.format),
        );
        c.id(&self.id.0, "id");
        c.name(&self.name, "name", true);
        c.check(
            self.tempo.len() == 1 && self.tempo[0].tick == Tick::ZERO,
            "tempo",
            ViolationKind::TempoMap,
        );
        c.check(
            self.meter.len() == 1 && self.meter[0].tick == Tick::ZERO,
            "meter",
            ViolationKind::MeterMap,
        );
        self.validate_transport(&mut c);
        c.mixer(&self.master, "master");
        self.validate_master_chain(&mut c);

        c.count(self.scenes.len(), c.limits.scenes, "scenes");
        for (id, scene) in &self.scenes {
            let path = format!("scenes/{id}");
            c.id(&id.0, &path);
            c.name(&scene.name, &path, false);
        }

        c.count(self.tracks.len(), c.limits.tracks, "tracks");
        let (mut notes, mut points) = (0usize, 0usize);
        for (id, track) in &self.tracks {
            let path = format!("tracks/{id}");
            c.id(&id.0, &path);
            self.validate_track(&mut c, track, &path, &mut notes, &mut points);
        }
        c.count(notes, c.limits.notes, "tracks");
        c.count(points, c.limits.points, "tracks");
        let counts = self.counts();
        c.count(counts.clips, c.limits.clips, "tracks");
        c.count(counts.placements, c.limits.placements, "tracks");
        c.count(counts.lanes, c.limits.lanes, "tracks");

        if c.out.is_empty() { Ok(()) } else { Err(c.out) }
    }

    fn validate_transport(&self, c: &mut Checker<'_>) {
        let t = &self.transport;
        if let Some(region) = t.loop_region {
            c.tick(region.start(), "transport/loop_region");
            c.tick(region.end(), "transport/loop_region");
        }
        c.check(
            !t.loop_enabled || t.loop_region.is_some(),
            "transport/loop_enabled",
            ViolationKind::LoopWithoutRegion,
        );
        c.bounded_gain(t.metronome.gain_db, "transport/metronome");
        c.check(
            t.metronome.count_in_bars <= MAX_COUNT_IN_BARS,
            "transport/metronome",
            ViolationKind::CountIn,
        );
    }

    /// The master's chain: effects only, built-in or module, with finite parameters.
    fn validate_master_chain<'p>(&'p self, c: &mut Checker<'p>) {
        c.count(
            self.master_devices.len(),
            c.limits.devices_per_track,
            "master",
        );
        for (id, device) in self.master_devices_in_order() {
            let path = format!("master/devices/{id}");
            c.id(&id.0, &path);
            c.check(
                device.role == DeviceRole::Effect
                    && !matches!(&device.source, DeviceSource::Builtin { id, .. } if id == CLIP_PLAYER),
                &path,
                ViolationKind::NotAnEffect,
            );
            match &device.source {
                DeviceSource::Builtin { id, .. } => {
                    c.check(is_key(id), &path, ViolationKind::BadKey)
                }
                DeviceSource::Module { name, .. } => {
                    c.name(name, &path, true);
                }
            }
            c.count(device.params.len(), c.limits.params_per_device, &path);
            for (param, value) in &device.params {
                c.check(is_key(param), &path, ViolationKind::BadKey);
                c.check(value.is_finite(), &path, ViolationKind::NotFinite);
            }
        }
    }

    fn validate_track<'p>(
        &'p self,
        c: &mut Checker<'p>,
        track: &'p Track,
        path: &str,
        notes: &mut usize,
        points: &mut usize,
    ) {
        c.name(&track.name, path, false);
        c.mixer(&track.mixer, path);

        // The chain.
        c.count(track.devices.len(), c.limits.devices_per_track, path);
        let chain = track.devices_in_order();
        let instruments = chain
            .iter()
            .filter(|(_, d)| d.role == DeviceRole::Instrument)
            .count();
        c.check(instruments <= 1, path, ViolationKind::TooManyInstruments);
        c.check(
            track.kind == TrackKind::Midi || instruments > 0,
            path,
            ViolationKind::MissingInstrument,
        );
        for (i, (id, device)) in chain.iter().enumerate() {
            let path = format!("{path}/devices/{id}");
            c.id(&id.0, &path);
            let is_player =
                matches!(&device.source, DeviceSource::Builtin { id, .. } if id == CLIP_PLAYER);
            if device.role == DeviceRole::Instrument {
                c.check(i == 0, &path, ViolationKind::InstrumentNotFirst);
                c.check(
                    is_player == (track.kind == TrackKind::Audio),
                    &path,
                    ViolationKind::WrongInstrument,
                );
            } else {
                c.check(!is_player, &path, ViolationKind::WrongInstrument);
            }
            match &device.source {
                DeviceSource::Builtin { id, .. } => {
                    c.check(is_key(id), &path, ViolationKind::BadKey)
                }
                DeviceSource::Module { name, .. } => {
                    c.name(name, &path, true);
                }
            }
            c.count(device.params.len(), c.limits.params_per_device, &path);
            for (param, value) in &device.params {
                c.check(is_key(param), &path, ViolationKind::BadKey);
                c.check(value.is_finite(), &path, ViolationKind::NotFinite);
            }
        }

        // Clips.
        c.count(track.clips.len(), c.limits.clips_per_track, path);
        let mut slots = HashSet::new();
        for (id, clip) in &track.clips {
            let path = format!("{path}/clips/{id}");
            c.id(&id.0, &path);
            c.name(&clip.name, &path, false);
            if let Some(scene) = &clip.slot {
                c.check(
                    self.scenes.contains_key(scene),
                    &path,
                    ViolationKind::UnknownScene,
                );
                c.check(slots.insert(scene), &path, ViolationKind::SlotTaken);
            }
            c.check(
                clip.content.is_midi() == (track.kind == TrackKind::Midi),
                &path,
                ViolationKind::KindMismatch,
            );
            validate_content(c, &clip.content, &path, notes);
        }

        // Arrangement.
        c.count(track.placements.len(), c.limits.placements_per_track, path);
        for (id, placement) in &track.placements {
            let path = format!("{path}/placements/{id}");
            c.id(&id.0, &path);
            validate_placement(c, placement, &track.clips, self.tempo(), &path);
        }

        // Automation.
        c.count(track.automation.len(), c.limits.lanes_per_track, path);
        for (id, lane) in &track.automation {
            let path = format!("{path}/automation/{id}");
            c.id(&id.0, &path);
            let pan = lane.target
                == AutomationTarget::Mixer {
                    param: MixerParam::Pan,
                };
            let gain = lane.target
                == AutomationTarget::Mixer {
                    param: MixerParam::Gain,
                };
            if let AutomationTarget::Device { device, param } = &lane.target {
                c.check(
                    track.devices.contains_key(device),
                    &path,
                    ViolationKind::UnknownDevice,
                );
                c.check(is_key(param), &path, ViolationKind::BadKey);
            }
            c.count(lane.points.len(), c.limits.points_per_lane, &path);
            *points = points.saturating_add(lane.points.len());
            for (tick, point) in &lane.points {
                c.tick(*tick, &path);
                c.check(point.value.is_finite(), &path, ViolationKind::NotFinite);
                if pan && point.value.is_finite() {
                    c.check(
                        (-1.0..=1.0).contains(&point.value),
                        &path,
                        ViolationKind::PanOutOfRange,
                    );
                }
                // A gain point is a level, never silence: `-inf` was refused above.
                if gain && point.value.is_finite() {
                    c.check(
                        (MIN_GAIN_DB..=MAX_GAIN_DB).contains(&point.value),
                        &path,
                        ViolationKind::GainOutOfRange,
                    );
                }
            }
        }
    }
}

fn validate_content(c: &mut Checker<'_>, content: &ClipContent, path: &str, notes: &mut usize) {
    match content {
        ClipContent::Midi {
            length,
            looping,
            notes: clip_notes,
        } => {
            let len = length.get();
            let length_ok = (1..=MAX_TICK).contains(&len);
            c.check(length_ok, path, ViolationKind::BadLength);
            if let Some(l) = looping {
                c.check(
                    0 <= l.start.get() && l.start < l.end && l.end.get() <= len,
                    path,
                    ViolationKind::BadLoop,
                );
            }
            c.count(clip_notes.len(), c.limits.notes_per_clip, path);
            *notes = notes.saturating_add(clip_notes.len());
            for n in clip_notes {
                let ok = n.fields_in_range() && n.length.get() > 0;
                c.check(ok, path, ViolationKind::BadNote);
                let inside = n.start.get() >= 0
                    && n.start
                        .get()
                        .checked_add(n.length.get())
                        .is_some_and(|end| end <= len);
                c.check(inside, path, ViolationKind::NoteOutsideClip);
            }
        }
        ClipContent::Audio {
            media,
            start,
            frames,
            gain_db,
            looping,
        } => {
            c.name(&media.name, path, false);
            c.check(
                (1..=MAX_CHANNELS).contains(&media.channels),
                path,
                ViolationKind::BadChannels,
            );
            c.check(frame_ok(media.frames), path, ViolationKind::BadLength);
            c.check(
                SAMPLE_RATES.contains(&media.sample_rate.hz()),
                path,
                ViolationKind::BadSampleRate,
            );
            c.check(
                frames.get() > 0 && frame_ok(*frames),
                path,
                ViolationKind::BadLength,
            );
            c.check(
                start
                    .get()
                    .checked_add(frames.get())
                    .is_some_and(|end| end <= media.frames.get()),
                path,
                ViolationKind::BadRange,
            );
            c.gain(*gain_db, path);
            if let Some(l) = looping {
                c.check(
                    l.start < l.end && l.end <= *frames,
                    path,
                    ViolationKind::BadLoop,
                );
            }
        }
    }
}

fn validate_placement(
    c: &mut Checker<'_>,
    placement: &Placement,
    clips: &BTreeMap<crate::ids::ClipId, Clip>,
    tempo: engine::time::Tempo,
    path: &str,
) {
    c.tick(placement.start, path);
    let Some(clip) = clips.get(&placement.clip) else {
        c.push(path, ViolationKind::UnknownClip);
        return;
    };
    match (&placement.span, &clip.content) {
        (
            Span::Ticks { length, offset },
            ClipContent::Midi {
                length: clip_len, ..
            },
        ) => {
            c.check(
                (1..=MAX_TICK).contains(&length.get()),
                path,
                ViolationKind::BadLength,
            );
            c.check(
                placement
                    .start
                    .get()
                    .checked_add(length.get())
                    .is_some_and(|end| end <= MAX_TICK),
                path,
                ViolationKind::BadPosition,
            );
            c.check(
                0 <= offset.get() && *offset < *clip_len,
                path,
                ViolationKind::BadRange,
            );
        }
        (Span::Frames { length, offset }, ClipContent::Audio { frames, media, .. }) => {
            c.check(
                length.get() > 0 && frame_ok(*length),
                path,
                ViolationKind::BadLength,
            );
            // The end tick, computed at the project's tempo (no tick length is stored).
            let end = placement
                .start
                .saturating_add(tempo.frames_to_ticks(*length, media.sample_rate));
            c.check(end.get() <= MAX_TICK, path, ViolationKind::BadPosition);
            c.check(*offset < *frames, path, ViolationKind::BadRange);
        }
        _ => c.push(path, ViolationKind::KindMismatch),
    }
}
