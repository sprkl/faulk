//! Hiccup, `faulk.hiccup`, displayed "Hiccup" (issue #689): captures a slice of the sound on
//! the transport's grid and repeats it in rhythm (a stutter at the end of a phrase, a snare
//! roll before a drop). A [`Device`] of role effect (CT-5).
//!
//! # Captures
//!
//! - **On the grid**: while the transport plays, every `interval` (1/16 note to 4 bars of the
//!   block's signature, counted from tick 0) is a grid point. A grid point at tick `B` lands at
//!   the frame that contains it, the host's rule for an event at a tick
//!   (`crate::transport::Segment::offset_of`): its fractional sample `s = (B - pos) × sr /
//!   ticks_per_second` belongs to the block when `s` is in `[-ε, frames - ε)` and lands at
//!   `floor(s + ε)` (ε = [`GRID_EPSILON_FRAMES`]: one shared epsilon, so a point on the edge of
//!   two blocks is taken once, never twice, never lost; a point less than ε before a frame
//!   boundary lands on the later frame, where the host's plain floor puts it on the earlier
//!   one).
//! - The grid runs whenever the clock runs, a count-in included (negative ticks: `n` is the
//!   ceiling rule above applied to negative positions, the draw takes it as is).
//! - **By chance**: a grid point captures when `draw(seed, n) % 100 < chance`, `n = B /
//!   interval` (an integer hash, [`draw`]): a pure function of the seed and of the grid
//!   position, so playback, the export, a seek, a loop and the WASM build give the same
//!   captures.
//! - **By hand**: `capture` going from off to on starts a capture at its frame (an automation
//!   lane places it on the grid if wanted); its repeats last while `capture` stays on, and end
//!   when it goes off. A value already on (a saved project, after a reset) starts nothing: a
//!   capture starts on the change. While a manual capture runs, the grid is ignored.
//!
//! A transport stop (the first block with the clock stopped after one with it running) ends
//! the capture that runs, grid or manual: nothing repeats on after the stop. A seek or a loop
//! wrap is not a stop: the window that runs plays on in frames, and the next grid point the
//! new position crosses captures.
//!
//! # A window
//!
//! A capture opens a **window** of `W` frames: `interval × gate / 100` at the tempo of the
//! block it starts in, and never past the next grid point (a manual window lasts while
//! `capture` is on). Its first `S` frames
//! (`size`, at that tempo, at most [`MAX_SLICE_SECONDS`]) are the **slice**: it plays as it
//! comes in and is recorded. Then the slice **repeats** until the window ends, each repeat
//! starting where the one before ended; `S` and `W` are fixed when the window opens (a tempo
//! change during a window does not move it). A gate shorter than the slice (`W <= S`) gives no
//! repeat: the slice plays as it comes and the window ends; the next slice is written after
//! the part recorded. Repeat `k` (from 1):
//!
//! - lasts `L_k`: `S` (fixed), `S × 0.75^(k-1)` rounded, at least `S / 8` (shorter), or `S`,
//!   `S / 2` or `S / 4` drawn from (seed, capture, k) (random);
//! - plays at the gain `(1 - decay)^k` (the decay set when it starts);
//! - reads the slice at the rate `2^(pitch / 12)` ([`PITCH_RATES`], linear interpolation): raised, it ends when
//!   the slice runs out (silence until the next repeat); lowered, it plays the slice's start;
//! - goes through the filter (a two-pole state-variable filter, Q 0.707): its cutoff for a
//!   repeat that starts `t` frames into the window is `open × (closed / open)^(t / W)` (t / W
//!   at most 1), low-pass from 20 kHz down to 200 Hz, high-pass from 20 Hz up to 2 kHz, kept
//!   below 0.45 × the sample rate. The cutoff glides to it over the repeat's fade (no step).
//!
//! `size`, `gate`, `interval` and the `filter`'s type apply to the next capture (the type
//! from its first repeat); `variation`, `decay` and `pitch` to the next repeat (at a repeat's start the signal is faded out, so a change never
//! clicks); `mode` and `mix` at once, through the dry gain's slew and the mix's 20 ms ramp.
//!
//! # Output
//!
//! `out = dry × g + wet × mix`, `wet` the repeats (and the tail of a window cut short) through
//! the filter, `g` the dry gain, which moves toward its target by at most `1 / F` per frame
//! (`F` = [`FADE_SECONDS`]), so it never steps:
//!
//! - **layer**: `g` is 1: the repeats play on top of the sound;
//! - **insert** (default): `g` is `1 - mix` while the repeats play and 1 elsewhere (back up
//!   over the last `F` frames of the window): at 100 % the sound is muted under the repeats;
//! - **repeats only**: `g` is `mix` while the slice plays and 0 elsewhere: only the captured
//!   slices and their repeats are heard.
//!
//! # Declicking
//!
//! Each repeat fades in over its first `f` frames and out over its last `f` (`f = min(F,
//! L_k / 4)`, at least one frame; linear, like the dry gain, so the crossfade of the dry and
//! of the first repeat sums to a constant). A fade adds at most `peak / f` to the step from a
//! sample to the next: with full fades (repeats of at least `4 F` frames) the steps at the
//! edges of a repeat stay within the input's own steps plus `peak / F`. A window cut short (a manual capture started or
//! ended while repeats play, a stop) keeps its repeat as a **tail** that fades out over `F`
//! frames while the new capture starts at its own frame.
//!
//! # Latency
//!
//! None: the slice is heard as it comes in, the repeats come after it is recorded. The
//! descriptor has nothing to declare.
//!
//! # Memory
//!
//! One ring per channel, allocated in the factory (`prepare`) for [`MAX_SLICE_SECONDS`] plus
//! two fades at the setup's rate (at most [`MAX_SAMPLE_RATE`]). Its write head moves only while
//! a slice is recorded, and a new slice is written after the previous one, so the tail of a
//! cut window (`F` frames) never reads what the new capture overwrites (the ring holds the
//! longest slice and two fades). [`Device::reset`] takes constant time: it ends the window and
//! the tail, the ring is not wiped (a slice is read only after its own capture).
//!
//! Nothing here allocates, locks or panics. The input is sanitized (a non-finite sample is 0,
//! a magnitude beyond 1e6 is clamped, below 1e-20 it is flushed), the output and the filter's
//! state are flushed of denormals.

use super::Builtin;
use crate::device::{
    AudioIo, BlockCtx, Device, DeviceDescriptor, DeviceRole, DeviceSetup, Event, MAX_SAMPLE_RATE,
    ParamDescriptor, Scale, Unit, event_frame,
};
use crate::msg::{DeviceKind, MsgError, ParamId, try_box};
use crate::param::Smoother;
use crate::time::{PPQ, TimeSignature};

/// Ids of the parameters.
pub const INTERVAL: u32 = 0;
pub const CHANCE: u32 = 1;
pub const SEED: u32 = 2;
pub const CAPTURE: u32 = 3;
pub const SIZE: u32 = 4;
pub const GATE: u32 = 5;
pub const VARIATION: u32 = 6;
pub const DECAY: u32 = 7;
pub const FILTER: u32 = 8;
pub const PITCH: u32 = 9;
pub const MODE: u32 = 10;
pub const MIX: u32 = 11;

/// The values of `interval`, from a 1/16 note to 4 bars.
pub const EVERY_1_16: u32 = 0;
pub const EVERY_1_8: u32 = 1;
pub const EVERY_1_4: u32 = 2;
pub const EVERY_1_2: u32 = 3;
pub const EVERY_BAR: u32 = 4;
pub const EVERY_2_BARS: u32 = 5;
pub const EVERY_4_BARS: u32 = 6;

/// The values of `size`, from a 1/64 note to a bar.
pub const SIZE_1_64: u32 = 0;
pub const SIZE_1_32: u32 = 1;
pub const SIZE_1_16T: u32 = 2;
pub const SIZE_1_16: u32 = 3;
pub const SIZE_1_8T: u32 = 4;
pub const SIZE_1_8: u32 = 5;
pub const SIZE_1_4: u32 = 6;
pub const SIZE_1_2: u32 = 7;
pub const SIZE_BAR: u32 = 8;

/// The values of `variation`.
pub const FIXED: u32 = 0;
pub const SHORTER: u32 = 1;
pub const RANDOM: u32 = 2;

/// The values of `filter`.
pub const FILTER_OFF: u32 = 0;
pub const LOW_PASS: u32 = 1;
pub const HIGH_PASS: u32 = 2;

/// The values of `mode`.
pub const LAYER: u32 = 0;
pub const INSERT: u32 = 1;
pub const REPEATS_ONLY: u32 = 2;

/// Longest slice, in seconds: the longest `size`, one bar, of 4/4 at the slowest tempo
/// (20 BPM). The ring holds a slice, never the interval, which needs no audio; a longer slice
/// (a bar of more than four quarter notes at 20 BPM) is shortened to it.
pub const MAX_SLICE_SECONDS: f32 = 12.0;
/// Length of the fades at the edges of a repeat and of the dry gain's slew, in seconds.
pub const FADE_SECONDS: f32 = 0.002;
/// The epsilon of the grid rule, in frames (module doc).
pub const GRID_EPSILON_FRAMES: f64 = 1e-6;
/// Largest transport position the grid runs at, in ticks (about 30 years at 999 BPM): beyond,
/// no grid point is looked for.
pub const MAX_GRID_TICKS: f64 = 1e12;
/// The ramp of `mix`, in seconds.
const MIX_SMOOTHING_SECONDS: f64 = 0.02;

/// The filter's sweeps: from the open cutoff at the window's start to the closed one at its end.
pub const LOW_PASS_OPEN_HZ: f32 = 20_000.0;
pub const LOW_PASS_CLOSED_HZ: f32 = 200.0;
pub const HIGH_PASS_OPEN_HZ: f32 = 20.0;
pub const HIGH_PASS_CLOSED_HZ: f32 = 2_000.0;
/// Highest cutoff, as a share of the sample rate.
const CUTOFF_NYQUIST_SHARE: f32 = 0.45;
/// Damping of the state-variable filter: Q = 1 / k = 0.707.
const DAMPING: f32 = core::f32::consts::SQRT_2;

/// Each repeat of `shorter` lasts this share of the one before, down to [`SHORTEST_SHARE`] of
/// the slice.
pub const SHORTER_RATIO: f64 = 0.75;
pub const SHORTEST_SHARE: f64 = 0.125;

/// The input is clamped to this magnitude; below [`DENORMAL_FLOOR`] it is flushed.
const INPUT_LIMIT: f32 = 1e6;
const DENORMAL_FLOOR: f32 = 1e-20;
/// Frames kept past the longest slice and its two fades.
const GUARD_FRAMES: usize = 4;

const VALUES_INTERVAL: &[&str] = &[
    "sixteenth",
    "eighth",
    "quarter",
    "half",
    "bar",
    "two_bars",
    "four_bars",
];
const VALUES_SIZE: &[&str] = &[
    "sixty_fourth",
    "thirty_second",
    "sixteenth_triplet",
    "sixteenth",
    "eighth_triplet",
    "eighth",
    "quarter",
    "half",
    "bar",
];

/// The rate a repeat reads its slice at, for each pitch from -12 to +12 semitones: `2^(p /
/// 12)` written out, so that no transcendental function (whose last bit differs between the
/// native targets and wasm32) decides a repeat's length in frames.
pub const PITCH_RATES: [f64; 25] = [
    0.5,
    0.5297315471796477,
    0.5612310241546865,
    0.5946035575013605,
    0.6299605249474366,
    0.6674199270850172,
    core::f64::consts::FRAC_1_SQRT_2,
    0.7491535384383408,
    0.7937005259840998,
    0.8408964152537145,
    0.8908987181403393,
    0.9438743126816935,
    1.0,
    1.0594630943592953,
    1.122462048309373,
    1.189207115002721,
    1.2599210498948732,
    1.3348398541700344,
    core::f64::consts::SQRT_2,
    1.4983070768766815,
    1.5874010519681994,
    1.681792830507429,
    1.7817974362806785,
    1.8877486253633868,
    2.0,
];

/// Frames of a fade at `sample_rate` (at least one).
fn fade_frames(sample_rate: f32) -> usize {
    ((FADE_SECONDS * sample_rate).ceil() as usize).max(1)
}

/// Frames of one ring at `sample_rate`: the longest slice, two fades and the guard.
fn ring_frames(sample_rate: f32) -> usize {
    (MAX_SLICE_SECONDS * sample_rate).ceil() as usize + 2 * fade_frames(sample_rate) + GUARD_FRAMES
}

/// Bytes of the two rings at [`MAX_SAMPLE_RATE`] (an upper bound of [`ring_frames`]).
const RINGS_BYTES: u64 = 2
    * ((MAX_SLICE_SECONDS as f64 * MAX_SAMPLE_RATE as f64) as u64
        + 2 * ((FADE_SECONDS as f64 * MAX_SAMPLE_RATE as f64) as u64 + 1)
        + GUARD_FRAMES as u64
        + 1)
    * size_of::<f32>() as u64;

/// The descriptor of Hiccup (CT-5): `faulk.hiccup`, displayed "Hiccup" (one English name,
/// never translated). The defaults are the values of [`Hiccup::new`]: a 1/16 slice captured at
/// every bar line and repeated for a quarter of the bar, inserted in place of the sound.
pub const DESCRIPTOR: DeviceDescriptor = DeviceDescriptor {
    kind: "faulk.hiccup",
    kind_id: DeviceKind(13),
    version: 1,
    name: "Hiccup",
    role: DeviceRole::Effect,
    memory_bytes: size_of::<Hiccup>() as u64 + RINGS_BYTES,
    midi_output: None,
    named_keys: &[],
    params: &[
        ParamDescriptor {
            id: ParamId(INTERVAL),
            key: "interval",
            description: "How often a slice may be captured, on the transport's grid: every 1/16 note to every 4 bars of the time signature. Captures happen only while the transport plays.",
            unit: Unit::None,
            min: 0.0,
            max: 6.0,
            default: EVERY_BAR as f64,
            scale: Scale::Linear,
            step: 1.0,
            smoothing_seconds: 0.0,
            values: VALUES_INTERVAL,
        },
        ParamDescriptor {
            id: ParamId(CHANCE),
            key: "chance",
            description: "Chance that a capture happens at each grid point, from 0 to 100 %. The draw depends only on the seed and the grid position: playback and export give the same captures.",
            unit: Unit::Percent,
            min: 0.0,
            max: 100.0,
            default: 100.0,
            scale: Scale::Linear,
            step: 1.0,
            smoothing_seconds: 0.0,
            values: &[],
        },
        ParamDescriptor {
            id: ParamId(SEED),
            key: "seed",
            description: "Seed of the chance and random length draws, from 0 to 1000: the same seed gives the same captures, another seed another sequence.",
            unit: Unit::None,
            min: 0.0,
            max: 1000.0,
            default: 0.0,
            scale: Scale::Linear,
            step: 1.0,
            smoothing_seconds: 0.0,
            values: &[],
        },
        ParamDescriptor {
            id: ParamId(CAPTURE),
            key: "capture",
            description: "Manual capture: turning it on captures at once and the repeats last while it stays on; off ends them. A capture starts only when the value turns on; a transport stop ends it.",
            unit: Unit::None,
            min: 0.0,
            max: 1.0,
            default: 0.0,
            scale: Scale::Linear,
            step: 1.0,
            smoothing_seconds: 0.0,
            values: &["off", "on"],
        },
        ParamDescriptor {
            id: ParamId(SIZE),
            key: "size",
            description: "Length of the repeated slice, as a note value of the project tempo: 1/64 to 1 bar, with triplets (T). A slice longer than 12 seconds is shortened to 12 seconds.",
            unit: Unit::None,
            min: 0.0,
            max: 8.0,
            default: SIZE_1_16 as f64,
            scale: Scale::Linear,
            step: 1.0,
            smoothing_seconds: 0.0,
            values: VALUES_SIZE,
        },
        ParamDescriptor {
            id: ParamId(GATE),
            key: "gate",
            description: "How long the repeats last after a capture, as a share of the interval, from 1 to 100 %. The slice plays first, then repeats until the gate ends.",
            unit: Unit::Percent,
            min: 1.0,
            max: 100.0,
            default: 25.0,
            scale: Scale::Linear,
            step: 0.0,
            smoothing_seconds: 0.0,
            values: &[],
        },
        ParamDescriptor {
            id: ParamId(VARIATION),
            key: "variation",
            description: "Length of each repeat. Fixed: the slice. Shorter: three quarters of the repeat before, down to an eighth of the slice. Random: the slice, half or a quarter of it, drawn from the seed.",
            unit: Unit::None,
            min: 0.0,
            max: 2.0,
            default: FIXED as f64,
            scale: Scale::Linear,
            step: 1.0,
            smoothing_seconds: 0.0,
            values: &["fixed", "shorter", "random"],
        },
        ParamDescriptor {
            id: ParamId(DECAY),
            key: "decay",
            description: "Level lost at each repeat, from 0 to 100 %: at 50 %, each repeat is half as loud as the one before. A change applies from the next repeat.",
            unit: Unit::Percent,
            min: 0.0,
            max: 100.0,
            default: 0.0,
            scale: Scale::Linear,
            step: 0.0,
            smoothing_seconds: 0.0,
            values: &[],
        },
        ParamDescriptor {
            id: ParamId(FILTER),
            key: "filter",
            description: "A filter that closes over the repeats. Low-pass: from 20 kHz down to 200 Hz at the end of the gate, darker at each repeat. High-pass: from 20 Hz up to 2 kHz, thinner at each repeat. A change applies from the next capture.",
            unit: Unit::None,
            min: 0.0,
            max: 2.0,
            default: FILTER_OFF as f64,
            scale: Scale::Linear,
            step: 1.0,
            smoothing_seconds: 0.0,
            values: &["off", "low_pass", "high_pass"],
        },
        ParamDescriptor {
            id: ParamId(PITCH),
            key: "pitch",
            description: "Pitch of the repeats, from -12 to +12 semitones, by playing the slice faster or slower. Raised, a repeat ends when the slice runs out. A change applies from the next repeat.",
            unit: Unit::Semitones,
            min: -12.0,
            max: 12.0,
            default: 0.0,
            scale: Scale::Linear,
            step: 1.0,
            smoothing_seconds: 0.0,
            values: &[],
        },
        ParamDescriptor {
            id: ParamId(MODE),
            key: "mode",
            description: "How the repeats meet the sound. Layer: on top of it. Insert: the sound is muted while the repeats play. Repeats only: only the captured slices and their repeats are heard.",
            unit: Unit::None,
            min: 0.0,
            max: 2.0,
            default: INSERT as f64,
            scale: Scale::Linear,
            step: 1.0,
            smoothing_seconds: 0.0,
            values: &["layer", "insert", "repeats_only"],
        },
        ParamDescriptor {
            id: ParamId(MIX),
            key: "mix",
            description: "Level of the repeats, from 0 to 100 %. In insert mode it also sets how much the sound is muted while they play.",
            unit: Unit::Percent,
            min: 0.0,
            max: 100.0,
            default: 100.0,
            scale: Scale::Linear,
            step: 0.0,
            smoothing_seconds: MIX_SMOOTHING_SECONDS,
            values: &[],
        },
    ],
};

/// Hiccup's line of the registry ([`super::REGISTRY`]).
pub const BUILTIN: Builtin = Builtin {
    descriptor: &DESCRIPTOR,
    factory,
};

/// Builds a Hiccup for `setup`, off the audio path: both rings are allocated here, with a
/// fallible allocation.
pub fn factory(setup: &DeviceSetup) -> Result<Box<dyn Device>, MsgError> {
    Ok(try_box(Hiccup::new(setup.sample_rate)?)?)
}

/// The length of an `interval` value in ticks under `signature`.
pub fn interval_ticks(interval: u32, signature: TimeSignature) -> i64 {
    let whole = 4 * PPQ;
    let bar = signature.ticks_per_bar().get();
    match interval {
        EVERY_1_16 => whole / 16,
        EVERY_1_8 => whole / 8,
        EVERY_1_4 => whole / 4,
        EVERY_1_2 => whole / 2,
        EVERY_BAR => bar,
        EVERY_2_BARS => 2 * bar,
        _ => 4 * bar,
    }
}

/// The length of a `size` value in ticks under `signature`.
pub fn size_ticks(size: u32, signature: TimeSignature) -> i64 {
    let whole = 4 * PPQ;
    match size {
        SIZE_1_64 => whole / 64,
        SIZE_1_32 => whole / 32,
        SIZE_1_16T => whole / 24,
        SIZE_1_16 => whole / 16,
        SIZE_1_8T => whole / 12,
        SIZE_1_8 => whole / 8,
        SIZE_1_4 => whole / 4,
        SIZE_1_2 => whole / 2,
        _ => signature.ticks_per_bar().get(),
    }
}

/// SplitMix64's finalizer: a well-mixed 64-bit hash of `z`.
fn mix64(z: u64) -> u64 {
    let mut z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// The deterministic draw of a seed, a capture key (the grid index of a grid capture) and a
/// repeat (0 for the capture's own chance): integer arithmetic only, the same on every target.
pub fn draw(seed: u32, key: u64, repeat: u32) -> u64 {
    mix64(mix64(mix64(u64::from(seed)) ^ key) ^ u64::from(repeat))
}

/// Whether the grid point of index `n` captures, for this seed and chance (0 to 100).
pub fn captures(seed: u32, n: i64, chance: u32) -> bool {
    draw(seed, n as u64, 0) % 100 < u64::from(chance)
}

fn sanitize(x: f32) -> f32 {
    if !x.is_finite() || x.abs() < DENORMAL_FLOOR {
        0.0
    } else {
        x.clamp(-INPUT_LIMIT, INPUT_LIMIT)
    }
}

fn flush(x: f32) -> f32 {
    if x.abs() < DENORMAL_FLOOR { 0.0 } else { x }
}

/// A ring of `frames` zeroed samples, or a typed error when the memory is refused.
fn ring(frames: usize) -> Result<Box<[f32]>, MsgError> {
    let mut v = Vec::new();
    v.try_reserve_exact(frames)
        .map_err(|_| MsgError::OutOfMemory {
            bytes: frames.saturating_mul(size_of::<f32>()),
        })?;
    v.resize(frames, 0.0);
    Ok(v.into_boxed_slice())
}

/// One repeat being played: where its slice is, how far it is, how it ends.
#[derive(Clone, Copy, Debug, Default)]
struct Repeat {
    /// Ring index of the slice's first frame.
    start: usize,
    /// Frames of the slice.
    slice: u32,
    /// Frames played of this repeat.
    u: u32,
    /// Frames this repeat sounds (its length, shortened when the slice runs out at a raised
    /// pitch or the window ends).
    end: u32,
    /// Frames of its fades (at least 1).
    fade: u32,
    rate: f64,
    gain: f32,
}

impl Repeat {
    /// The next frame of this repeat, faded at its edges, and one frame on.
    fn next(&mut self, left: &[f32], right: &[f32]) -> (f32, f32) {
        if self.u >= self.end {
            return (0.0, 0.0);
        }
        let u = self.u;
        self.u += 1;
        let fade = self.fade as f32;
        let env = (u as f32 / fade).min((self.end - u) as f32 / fade).min(1.0) * self.gain;
        let p = f64::from(u) * self.rate;
        let len = left.len();
        let whole = (p as usize).min(self.slice.saturating_sub(1) as usize);
        let frac = ((p - whole as f64) as f32).clamp(0.0, 1.0);
        let a = (self.start + whole) % len;
        let b = if whole + 1 < self.slice as usize {
            (a + 1) % len
        } else {
            a
        };
        let read = |ring: &[f32]| ring[a] + (ring[b] - ring[a]) * frac;
        (read(left) * env, read(right) * env)
    }
}

/// A capture and its repeats.
#[derive(Clone, Copy, Debug, Default)]
struct Window {
    active: bool,
    manual: bool,
    /// Frames since the capture.
    t: u64,
    /// Ring index of the slice's first frame.
    start: usize,
    /// Frames of the slice.
    slice: u32,
    /// Frame (from the capture) at which the window ends; `u64::MAX` for a manual one.
    end: u64,
    /// `interval × gate` in frames: the filter's sweep length.
    nominal: u64,
    /// The key of the random draws.
    key: u64,
    /// Repeats started.
    k: u32,
    /// Frame (from the capture) at which the next repeat starts.
    next_at: u64,
    /// The length of the last repeat (`shorter`), in frames.
    length: f64,
    /// The gain of the last repeat.
    gain: f32,
    repeat: Repeat,
}

/// The two integrator states of one channel of the state-variable filter.
#[derive(Clone, Copy, Debug, Default)]
struct Channel {
    ic1: f32,
    ic2: f32,
}

impl Channel {
    /// One sample: the `(low, high)` outputs.
    #[inline]
    fn tick(&mut self, x: f32, a1: f32, a2: f32, a3: f32) -> (f32, f32) {
        let v3 = x - self.ic2;
        let v1 = a1 * self.ic1 + a2 * v3;
        let v2 = self.ic2 + a2 * self.ic1 + a3 * v3;
        self.ic1 = flush(2.0 * v1 - self.ic1);
        self.ic2 = flush(2.0 * v2 - self.ic2);
        (v2, x - DAMPING * v1 - v2)
    }
}

#[derive(Debug)]
pub struct Hiccup {
    sample_rate: f32,
    left: Box<[f32]>,
    right: Box<[f32]>,
    /// The ring's write head: moves only while a slice is recorded.
    write: usize,
    /// The longest slice, in frames.
    max_slice: u32,
    /// Frames of a fade.
    fade: u32,
    interval: u32,
    chance: u32,
    seed: u32,
    capture: bool,
    /// A change of `capture` not yet applied: applied at the frame it was set.
    pending: Option<bool>,
    size: u32,
    gate: f64,
    variation: u32,
    /// Share of the level kept from a repeat to the next.
    keep: f32,
    filter: u32,
    rate: f64,
    mode: u32,
    mix: Smoother,
    mix_frames: u32,
    /// Samples per tick and the signature of the last block.
    samples_per_tick: f64,
    signature: TimeSignature,
    was_playing: bool,
    window: Window,
    tail: Repeat,
    tail_left: u32,
    dry: f32,
    /// The filter type of the repeat that plays (latched at its start).
    active_filter: u32,
    /// The filter's `g` (`tan(π fc / sr)`): its value, its step per frame and the frames left.
    g: f32,
    g_step: f32,
    g_left: u32,
    coefficients: (f32, f32, f32),
    filter_l: Channel,
    filter_r: Channel,
}

impl Hiccup {
    /// A Hiccup at its defaults, with its rings allocated for `sample_rate`.
    pub fn new(sample_rate: f32) -> Result<Self, MsgError> {
        let sample_rate = if sample_rate.is_finite() && sample_rate >= 8_000.0 {
            sample_rate
        } else {
            48_000.0
        };
        // Above the rate the descriptor declares its memory for, the rings stop at that size:
        // the longest slice is shorter, the times stay right.
        let sized = sample_rate.min(MAX_SAMPLE_RATE);
        let frames = ring_frames(sized);
        let fade = fade_frames(sample_rate) as u32;
        let default = |id: u32| DESCRIPTOR.param(ParamId(id)).map_or(0.0, |d| d.default);
        let mix_frames = DESCRIPTOR
            .param(ParamId(MIX))
            .map_or(0, |d| d.smoothing_frames(sample_rate));
        let tempo = crate::time::Tempo::default();
        let mut h = Self {
            sample_rate,
            left: ring(frames)?,
            right: ring(frames)?,
            write: 0,
            // Two fades at the real rate stay free beyond the longest slice (the tail of a cut
            // window never meets the next capture's writes), whatever the rate.
            max_slice: frames
                .saturating_sub(2 * fade as usize + GUARD_FRAMES)
                .max(1) as u32,
            fade,
            interval: default(INTERVAL) as u32,
            chance: default(CHANCE) as u32,
            seed: default(SEED) as u32,
            capture: false,
            pending: None,
            size: default(SIZE) as u32,
            gate: default(GATE),
            variation: default(VARIATION) as u32,
            keep: 1.0 - default(DECAY) as f32 / 100.0,
            filter: default(FILTER) as u32,
            rate: 1.0,
            mode: default(MODE) as u32,
            mix: Smoother::new(default(MIX) as f32 / 100.0),
            mix_frames,
            samples_per_tick: f64::from(sample_rate) / tempo.ticks_per_second(),
            signature: TimeSignature::default(),
            was_playing: false,
            window: Window::default(),
            tail: Repeat::default(),
            tail_left: 0,
            dry: 1.0,
            active_filter: FILTER_OFF,
            g: 0.0,
            g_step: 0.0,
            g_left: 0,
            coefficients: (1.0, 0.0, 0.0),
            filter_l: Channel::default(),
            filter_r: Channel::default(),
        };
        h.dry = h.base_target();
        h.g = h.g_of(LOW_PASS_OPEN_HZ);
        h.coefficients = coefficients(h.g);
        Ok(h)
    }

    /// The longest slice this instance holds, in frames.
    pub fn max_slice_frames(&self) -> u32 {
        self.max_slice
    }

    fn set_value(&mut self, id: ParamId, value: f32) {
        let Some(desc) = DESCRIPTOR.param(id) else {
            return;
        };
        let Some(value) = desc.sanitize(value) else {
            return;
        };
        match id.0 {
            INTERVAL => self.interval = value as u32,
            CHANCE => self.chance = value as u32,
            SEED => self.seed = value as u32,
            CAPTURE => {
                let on = value >= 0.5;
                if on != self.capture {
                    self.capture = on;
                    self.pending = Some(on);
                }
            }
            SIZE => self.size = value as u32,
            GATE => self.gate = f64::from(value),
            VARIATION => self.variation = value as u32,
            DECAY => self.keep = 1.0 - value / 100.0,
            FILTER => self.filter = value as u32,
            PITCH => self.rate = PITCH_RATES[(value + 12.0).clamp(0.0, 24.0) as usize],
            MODE => self.mode = value as u32,
            MIX => self.mix.set(value / 100.0, self.mix_frames),
            _ => {}
        }
    }

    /// The dry gain's target outside a window.
    fn base_target(&self) -> f32 {
        if self.mode == REPEATS_ONLY { 0.0 } else { 1.0 }
    }

    /// The filter's `g` for a cutoff, kept below the share of the sample rate.
    fn g_of(&self, hz: f32) -> f32 {
        let top = CUTOFF_NYQUIST_SHARE * self.sample_rate;
        let hz = hz.clamp(10.0, top);
        (core::f32::consts::PI * hz / self.sample_rate).tan()
    }

    /// Ends the window that runs: a repeat that sounds goes on as the tail, fading out over a
    /// fade. The dry gain then heads back to its target outside a window.
    fn cut(&mut self) {
        if self.window.active && self.window.k > 0 && self.window.repeat.u < self.window.repeat.end
        {
            self.tail = self.window.repeat;
            self.tail_left = self.fade;
        }
        self.window.active = false;
    }

    /// Opens a window at the frame about to be rendered; a grid window ends at the latest
    /// `until_next` frames on, where the next grid point lands (at a gate of 100 % the rounding
    /// of `W` never makes two windows overlap).
    fn open(&mut self, manual: bool, key: u64, until_next: u64) {
        self.cut();
        let spt = self.samples_per_tick;
        let ticks = size_ticks(self.size, self.signature) as f64;
        let slice = (ticks * spt).round().clamp(1.0, f64::from(self.max_slice)) as u32;
        let nominal = (interval_ticks(self.interval, self.signature) as f64 * self.gate / 100.0
            * spt)
            .round()
            .max(1.0);
        let nominal = if nominal.is_finite() {
            nominal.min(1e15) as u64
        } else {
            1
        };
        self.window = Window {
            active: true,
            manual,
            t: 0,
            start: self.write,
            slice,
            end: if manual {
                u64::MAX
            } else {
                nominal.min(until_next.max(1))
            },
            nominal,
            key,
            k: 0,
            next_at: u64::from(slice),
            length: f64::from(slice),
            gain: 1.0,
            repeat: Repeat::default(),
        };
    }

    /// Starts the next repeat of the window (at `window.t == window.next_at`).
    fn next_repeat(&mut self) {
        let w = &mut self.window;
        w.k = w.k.saturating_add(1);
        let slice = f64::from(w.slice);
        let length = match self.variation {
            SHORTER => {
                if w.k > 1 {
                    w.length *= SHORTER_RATIO;
                }
                w.length
                    .round()
                    .max((slice * SHORTEST_SHARE).round())
                    .max(1.0)
            }
            RANDOM => {
                let shift = draw(self.seed, w.key, w.k) % 3;
                (f64::from(w.slice >> shift)).max(1.0)
            }
            _ => slice,
        };
        let length = length.min(slice) as u64;
        w.gain = flush(w.gain * self.keep.clamp(0.0, 1.0));
        let rate = self.rate;
        let available = if rate > 1.0 {
            ((slice / rate).floor() as u64).max(1)
        } else {
            length
        };
        let left = w.end.saturating_sub(w.t);
        let end = length.min(available).min(left).min(u64::from(u32::MAX)) as u32;
        let fade = (self.fade).min((length / 4) as u32).max(1);
        w.repeat = Repeat {
            start: w.start,
            slice: w.slice,
            u: 0,
            end,
            fade,
            rate,
            gain: w.gain,
        };
        w.next_at = w.t.saturating_add(length.max(1));
        // The filter's type is latched at a window's first repeat: the wet path was silent for
        // the whole slice, so the filter's state has died down and switching its output never
        // steps; between two repeats only its cutoff moves (gliding over the fade).
        if w.k == 1 {
            self.active_filter = self.filter;
        }
        let x = (w.t as f64 / w.nominal.max(1) as f64).min(1.0) as f32;
        let (open, closed) = match self.active_filter {
            HIGH_PASS => (HIGH_PASS_OPEN_HZ, HIGH_PASS_CLOSED_HZ),
            _ => (LOW_PASS_OPEN_HZ, LOW_PASS_CLOSED_HZ),
        };
        let target = self.g_of(open * (closed / open).powf(x));
        self.g_left = fade;
        self.g_step = (target - self.g) / fade as f32;
    }

    /// Renders the frames of `left` and `right` (same length).
    fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        let len = self.left.len();
        let slew = 1.0 / self.fade as f32;
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            let mix = self.mix.next_value().clamp(0.0, 1.0);
            let (dry_l, dry_r) = (sanitize(*l), sanitize(*r));
            let mut target = self.base_target();
            let (mut wet_l, mut wet_r) = (0.0f32, 0.0f32);
            if self.window.active {
                let t = self.window.t;
                let fade_end = self.window.end.saturating_sub(u64::from(self.fade));
                if t < u64::from(self.window.slice) {
                    // The slice: heard as it comes in, recorded.
                    self.left[self.write] = dry_l;
                    self.right[self.write] = dry_r;
                    self.write = (self.write + 1) % len;
                    target = match self.mode {
                        REPEATS_ONLY if t < fade_end => mix,
                        REPEATS_ONLY => 0.0,
                        _ => 1.0,
                    };
                } else {
                    if t >= self.window.next_at {
                        self.next_repeat();
                    }
                    let (a, b) = self.window.repeat.next(&self.left, &self.right);
                    wet_l = a;
                    wet_r = b;
                    target = match self.mode {
                        LAYER => 1.0,
                        INSERT if t < fade_end => 1.0 - mix,
                        INSERT => 1.0,
                        _ => 0.0,
                    };
                }
                self.window.t = t + 1;
                if self.window.t >= self.window.end {
                    self.window.active = false;
                }
            }
            if self.tail_left > 0 {
                let share = self.tail_left as f32 / self.fade as f32;
                let (a, b) = self.tail.next(&self.left, &self.right);
                wet_l += a * share;
                wet_r += b * share;
                self.tail_left -= 1;
            }
            // The filter runs on every frame (its state stays continuous); its type and its
            // cutoff change only at a repeat's start, where the repeats are faded out.
            if self.g_left > 0 {
                self.g += self.g_step;
                self.g_left -= 1;
                self.coefficients = coefficients(self.g);
            }
            let (a1, a2, a3) = self.coefficients;
            let (low_l, high_l) = self.filter_l.tick(wet_l, a1, a2, a3);
            let (low_r, high_r) = self.filter_r.tick(wet_r, a1, a2, a3);
            let (wet_l, wet_r) = match self.active_filter {
                LOW_PASS => (low_l, low_r),
                HIGH_PASS => (high_l, high_r),
                _ => (wet_l, wet_r),
            };
            self.dry += (target - self.dry).clamp(-slew, slew);
            *l = flush(dry_l * self.dry + wet_l * mix);
            *r = flush(dry_r * self.dry + wet_r * mix);
        }
    }

    /// Applies a change of `capture` at the frame about to be rendered; `position` is the
    /// transport's position there, in ticks (the key of a manual capture's draws).
    fn apply_pending(&mut self, position: f64) {
        match self.pending.take() {
            Some(true) => {
                let tick = if position.is_finite() {
                    position.floor().clamp(-9e15, 9e15) as i64
                } else {
                    0
                };
                self.open(true, (tick as u64) ^ (1 << 63), u64::MAX);
            }
            Some(false) if self.window.active && self.window.manual => self.cut(),
            _ => {}
        }
    }
}

/// The filter's coefficients for `g`.
fn coefficients(g: f32) -> (f32, f32, f32) {
    let a1 = 1.0 / (1.0 + g * (g + DAMPING));
    let a2 = g * a1;
    (a1, a2, g * a2)
}

/// The grid of a block: where its grid points land.
#[derive(Clone, Copy, Debug)]
struct Grid {
    position: f64,
    samples_per_tick: f64,
    interval: i64,
    frames: usize,
}

impl Grid {
    /// The fractional sample of grid point `n`, from the block's start.
    fn sample(&self, n: i64) -> f64 {
        ((n as f64) * self.interval as f64 - self.position) * self.samples_per_tick
    }

    /// The first grid point whose sample is at or after `frame` (with the epsilon).
    fn first_at_or_after(&self, frame: usize) -> i64 {
        let threshold = frame as f64 - GRID_EPSILON_FRAMES;
        let guess = (self.position + threshold / self.samples_per_tick) / self.interval as f64;
        let mut n = if guess.is_finite() {
            guess.ceil().clamp(-1e15, 1e15) as i64
        } else {
            0
        };
        // The guess is within one of the answer (the position is bounded by
        // [`MAX_GRID_TICKS`]): settle it by the rule itself, in a bounded number of steps.
        for _ in 0..4 {
            if self.sample(n - 1) >= threshold {
                n -= 1;
            }
        }
        for _ in 0..4 {
            if self.sample(n) < threshold {
                n += 1;
            }
        }
        n
    }

    /// The frame grid point `n` lands at, or `None` when it is not in the block.
    fn frame_of(&self, n: i64) -> Option<usize> {
        let s = self.sample(n);
        if s.is_finite() && s < self.frames as f64 - GRID_EPSILON_FRAMES {
            Some(((s + GRID_EPSILON_FRAMES).floor().max(0.0) as usize).min(self.frames - 1))
        } else {
            None
        }
    }
}

impl Device for Hiccup {
    fn descriptor(&self) -> &'static DeviceDescriptor {
        &DESCRIPTOR
    }

    /// Renders the frames between events and grid points; at one frame the events apply first,
    /// then the grid point (a value set at the frame of a capture applies to it).
    fn process(&mut self, ctx: &BlockCtx, events: &[Event], io: &mut AudioIo<'_>) {
        let frames = (ctx.frames as usize).min(io.left.len()).min(io.right.len());
        let transport = ctx.transport;
        let tps = transport.tempo.ticks_per_second();
        let spt = f64::from(self.sample_rate) / tps;
        if spt.is_finite() && spt > 0.0 {
            self.samples_per_tick = spt;
        }
        self.signature = transport.signature;
        if self.was_playing && !transport.playing {
            self.cut();
        }
        self.was_playing = transport.playing;
        let position = if transport.position_ticks.is_finite() {
            transport.position_ticks
        } else {
            0.0
        };
        let mut grid = Grid {
            position,
            samples_per_tick: self.samples_per_tick,
            interval: interval_ticks(self.interval, self.signature).max(1),
            frames,
        };
        // A position beyond any song (a broken clock) has no grid: the search for its grid
        // point would not end in a bounded number of steps.
        let playing = transport.playing && frames > 0 && position.abs() < MAX_GRID_TICKS;
        let mut n = if playing {
            grid.first_at_or_after(0)
        } else {
            0
        };
        let position_at = |grid: &Grid, frame: usize| {
            if transport.playing {
                grid.position + frame as f64 / grid.samples_per_tick
            } else {
                grid.position
            }
        };
        self.apply_pending(position_at(&grid, 0));
        let mut cursor = 0;
        let mut next_event = 0;
        loop {
            let event_at = events
                .get(next_event)
                .map(|e| event_frame(e.offset(), cursor, frames));
            // A grid point passed while a manual capture held the grid never fires late.
            while playing && grid.sample(n) < cursor as f64 - GRID_EPSILON_FRAMES {
                n += 1;
            }
            let grid_at = if playing && !(self.window.active && self.window.manual) {
                grid.frame_of(n).map(|at| at.max(cursor))
            } else {
                None
            };
            let event_first = match (event_at, grid_at) {
                (Some(at), Some(g)) => at <= g,
                (Some(_), None) => true,
                _ => false,
            };
            match (event_at, grid_at) {
                (Some(at), _) if event_first => {
                    if at > cursor {
                        self.render(&mut io.left[cursor..at], &mut io.right[cursor..at]);
                        cursor = at;
                    }
                    if let Event::ParamValue { id, value, .. } = events[next_event] {
                        let interval = self.interval;
                        self.set_value(id, value);
                        if playing && self.interval != interval {
                            grid.interval = interval_ticks(self.interval, self.signature).max(1);
                            n = grid.first_at_or_after(cursor);
                        }
                    }
                    self.apply_pending(position_at(&grid, cursor));
                    next_event += 1;
                }
                (_, Some(at)) => {
                    if at > cursor {
                        self.render(&mut io.left[cursor..at], &mut io.right[cursor..at]);
                        cursor = at;
                    }
                    if captures(self.seed, n, self.chance) {
                        let next = grid.sample(n + 1) + GRID_EPSILON_FRAMES;
                        let here = grid.sample(n) + GRID_EPSILON_FRAMES;
                        let until_next = if next.is_finite() && here.is_finite() {
                            (next.floor() - here.floor()).clamp(1.0, 1e15) as u64
                        } else {
                            u64::MAX
                        };
                        self.open(false, n as u64, until_next);
                    }
                    n += 1;
                }
                _ => break,
            }
        }
        if frames > cursor {
            self.render(&mut io.left[cursor..frames], &mut io.right[cursor..frames]);
        }
    }

    fn set_param(&mut self, id: ParamId, value: f32) {
        self.set_value(id, value);
    }

    /// Ends the window and its tail, empties the filter, snaps the dry gain and the mix; the
    /// parameter values are kept, and a `capture` left on starts nothing. Constant time: the
    /// rings are not touched.
    fn reset(&mut self) {
        self.window.active = false;
        self.tail_left = 0;
        self.pending = None;
        self.was_playing = false;
        self.mix.snap();
        self.dry = self.base_target();
        self.filter_l = Channel::default();
        self.filter_r = Channel::default();
        self.g += self.g_step * self.g_left as f32;
        self.g_left = 0;
        self.coefficients = coefficients(self.g);
    }
}
