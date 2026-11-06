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
//! The engine-facing limits are the engine's own (`engine::msg::limits`, CT-3 #39): the
//! document takes them from there where they coincide, and the `const` block at the end
//! checks that no document limit exceeds the engine's, so that a project the document
//! accepts is never refused by the engine on a count.

/// Latest tick a position or an end may reach: 2^40 ticks, the bound under which CT-2
/// guarantees an exact tick → sample → tick round trip (about 18 years at 120 BPM).
pub const MAX_TICK: i64 = 1 << 40;

/// Largest frame index or frame count in an audio clip: 2^40 (about 260 days at 48 kHz).
pub const MAX_FRAME: u64 = 1 << 40;

/// Most bytes one audio file may decode to (#780), counted as its PCM cache counts them:
/// frames × channels × 4 (`f32`). 16 GiB, the most a whole project's `.faulkpack` carries
/// (`file::pack::PackLimits::max_total`): a file that compresses very well (a FLAC of
/// silence, a low-rate Opus) would otherwise fill the disk with its decoded copy. An hour
/// of stereo at 192 kHz (5.5 GB) or of 7.1 at 96 kHz (11.1 GB) fits; an hour of 16 channels
/// at 96 kHz (22.1 GB) does not. A WAV or AIFF read directly (no cache) never reaches it:
/// its 32-bit chunk sizes hold less than 4 GiB of samples of 16 bits or more.
pub const MAX_DECODED_BYTES: u64 = 16 << 30;

/// Longest name (project, track, scene, clip, module, audio file), in characters.
pub const MAX_NAME_CHARS: usize = 200;

/// Longest identifier string inside the document (parameter id, built-in device id), in
/// bytes. A module's name is a display name ([`MAX_NAME_CHARS`]).
pub const MAX_KEY_LEN: usize = 128;

/// Most tracks in a project.
pub const MAX_TRACKS: usize = engine::msg::limits::MAX_TRACKS as usize;

/// Most scenes in a project.
pub const MAX_SCENES: usize = engine::msg::limits::MAX_SCENES as usize;
/// Most section markers of a project (#473): the page draws them all on the ruler.
pub const MAX_MARKERS: usize = 1024;

/// Most devices in one track's chain.
pub const MAX_DEVICES_PER_TRACK: usize = 1_000;

/// Most parameters stored for one device.
pub const MAX_PARAMS_PER_DEVICE: usize = 10_000;

/// Most clips in one track.
pub const MAX_CLIPS_PER_TRACK: usize = 100_000;

/// Most placements in one track's arrangement.
pub const MAX_PLACEMENTS_PER_TRACK: usize = 100_000;

/// Most automation lanes in one track: as many as the engine plays on one track (AM-2).
pub const MAX_LANES_PER_TRACK: usize = engine::automation::LANES_PER_TRACK;

/// Most notes in one MIDI clip.
pub const MAX_NOTES_PER_CLIP: usize = 1_000_000;

/// Clips in the whole project, every track together: the engine's clip slots (PR-3).
pub const MAX_CLIPS: usize = engine::msg::limits::MAX_CLIPS as usize;

/// Placements in the whole project, every track together: the engine's placement slots.
pub const MAX_PLACEMENTS: usize = engine::msg::limits::MAX_PLACEMENTS as usize;

/// Automation lanes in the whole project, every track together: the engine's lane slots.
pub const MAX_LANES: usize = engine::msg::limits::MAX_LANES as usize;

/// Most points in one automation lane.
pub const MAX_POINTS_PER_LANE: usize = engine::msg::limits::MAX_POINTS_PER_LANE as usize;

/// Most notes in the whole project.
pub const MAX_NOTES: usize = engine::msg::limits::MAX_NOTES as usize;

/// Most automation points in the whole project.
pub const MAX_POINTS: usize = engine::msg::limits::MAX_AUTOMATION_POINTS as usize;

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
    pub clips: usize,
    pub placements: usize,
    pub lanes: usize,
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
        clips: MAX_CLIPS,
        placements: MAX_PLACEMENTS,
        lanes: MAX_LANES,
    };
}

// No document limit exceeds the engine's (CT-3): the project never accepts a count that a
// message would then be refused for.
const _: () = {
    use engine::msg::limits as engine;
    assert!(MAX_TRACKS <= engine::MAX_TRACKS as usize);
    assert!(MAX_SCENES <= engine::MAX_SCENES as usize);
    assert!(MAX_DEVICES_PER_TRACK <= engine::MAX_DEVICES_PER_TRACK as usize);
    assert!(MAX_CLIPS_PER_TRACK <= engine::MAX_CLIPS as usize);
    assert!(MAX_PLACEMENTS_PER_TRACK <= engine::MAX_PLACEMENTS_PER_TRACK as usize);
    assert!(MAX_LANES_PER_TRACK <= engine::MAX_LANES as usize);
    assert!(MAX_NOTES_PER_CLIP <= engine::MAX_NOTES_PER_CLIP as usize);
    assert!(MAX_POINTS_PER_LANE <= engine::MAX_POINTS_PER_LANE as usize);
    assert!(MAX_NOTES <= engine::MAX_NOTES as usize);
    assert!(MAX_CLIPS <= engine::MAX_CLIPS as usize);
    assert!(MAX_PLACEMENTS <= engine::MAX_PLACEMENTS as usize);
    assert!(MAX_LANES <= engine::MAX_LANES as usize);
    assert!(MAX_POINTS <= engine::MAX_AUTOMATION_POINTS as usize);
};
