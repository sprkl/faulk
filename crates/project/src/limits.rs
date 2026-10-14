//! Safety limits of the project document (founder's decision 7: no visible cap).
//!
//! These are not product limits: they are set far above what a person or an agent builds
//! (the load bench QA-7 targets 100 tracks, 200 000 notes and 6 million automation points
//! in a whole project), and exist only so that a corrupt or hostile file cannot make the
//! page build structures without bound or overflow a position. They are checked once the
//! document is read: reading itself holds at most what the input's size allows. [`crate::Project::validate`]
//! refuses a project beyond them, so that a command is refused with a typed error before
//! it reaches the engine, which never fails in real time (MVP breakdown § 2.3).
//!
//! The engine-facing totals (tracks, notes, points) are the safety limit CT-3 (#39) sets
//! for the engine; until it lands they are the example numbers of § 2.3 (4 096 tracks,
//! 16 777 216 notes). They must never exceed the engine's: whichever of PR-1 and CT-3
//! lands second aligns them.

/// Latest tick a position or an end may reach: 2^40 ticks, the bound under which CT-2
/// guarantees an exact tick → sample → tick round trip (about 18 years at 120 BPM).
pub const MAX_TICK: i64 = 1 << 40;

/// Largest frame index or frame count in an audio clip: 2^40 (about 260 days at 48 kHz).
pub const MAX_FRAME: u64 = 1 << 40;

/// Longest name (project, track, scene, clip, module, audio file), in characters.
pub const MAX_NAME_CHARS: usize = 200;

/// Longest identifier string inside the document (parameter id, built-in device id), in
/// bytes. A module's name is a display name ([`MAX_NAME_CHARS`]).
pub const MAX_KEY_LEN: usize = 128;

/// Most tracks in a project.
pub const MAX_TRACKS: usize = 4_096;

/// Most scenes in a project.
pub const MAX_SCENES: usize = 4_096;

/// Most devices in one track's chain.
pub const MAX_DEVICES_PER_TRACK: usize = 1_000;

/// Most parameters stored for one device.
pub const MAX_PARAMS_PER_DEVICE: usize = 10_000;

/// Most clips in one track.
pub const MAX_CLIPS_PER_TRACK: usize = 100_000;

/// Most placements in one track's arrangement.
pub const MAX_PLACEMENTS_PER_TRACK: usize = 100_000;

/// Most automation lanes in one track.
pub const MAX_LANES_PER_TRACK: usize = 10_000;

/// Most notes in one MIDI clip.
pub const MAX_NOTES_PER_CLIP: usize = 1_000_000;

/// Most points in one automation lane.
pub const MAX_POINTS_PER_LANE: usize = 16_777_216;

/// Most notes in the whole project.
pub const MAX_NOTES: usize = 16_777_216;

/// Most automation points in the whole project.
pub const MAX_POINTS: usize = 16_777_216;

/// Most channels in an audio file.
pub const MAX_CHANNELS: u16 = 64;

/// Most count-in bars of the metronome.
pub const MAX_COUNT_IN_BARS: u8 = 16;

/// Lowest and highest sample rate of an audio file, in hertz.
pub const SAMPLE_RATES: std::ops::RangeInclusive<u32> = 8_000..=768_000;

/// The count limits [`crate::Project::validate_with`] applies. [`Limits::SAFETY`] holds
/// the constants above and is what [`crate::Project::validate`] uses; smaller limits let
/// a test reach each count check without building millions of elements.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub tracks: usize,
    pub scenes: usize,
    pub devices_per_track: usize,
    pub params_per_device: usize,
    pub clips_per_track: usize,
    pub placements_per_track: usize,
    pub lanes_per_track: usize,
    pub notes_per_clip: usize,
    pub points_per_lane: usize,
    pub notes: usize,
    pub points: usize,
}

impl Limits {
    /// The safety limits of the document.
    pub const SAFETY: Limits = Limits {
        tracks: MAX_TRACKS,
        scenes: MAX_SCENES,
        devices_per_track: MAX_DEVICES_PER_TRACK,
        params_per_device: MAX_PARAMS_PER_DEVICE,
        clips_per_track: MAX_CLIPS_PER_TRACK,
        placements_per_track: MAX_PLACEMENTS_PER_TRACK,
        lanes_per_track: MAX_LANES_PER_TRACK,
        notes_per_clip: MAX_NOTES_PER_CLIP,
        points_per_lane: MAX_POINTS_PER_LANE,
        notes: MAX_NOTES,
        points: MAX_POINTS,
    };
}
