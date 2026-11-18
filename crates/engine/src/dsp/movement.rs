//! The movement of a device's parameter: a tempo LFO ([`super::lfo`]) and an envelope follower
//! ([`super::envelope_follower`]) turned into a per-frame modulation in **octaves**, one per
//! channel, with the eleven parameters that set them. The Filter (#686) moves its cutoff with
//! it, Ripple (#688) its notches; the block is the same, its descriptors are built by the
//! functions of this module, so ranges, defaults, units and value keys are written once.
//!
//! # The modulation of a frame
//!
//! [`Movement::next`] takes the level of the device's input (a rectified peak) and returns, for
//! the left and the right channel, `swing + env * amount` octaves:
//!
//! - **LFO**: `lfo_shape` (sine, triangle, saw up, saw down, square, random), `lfo_sync` free
//!   (`lfo_rate` in hertz) or synced (`lfo_division`, a note value, its phase the transport's
//!   position: a cycle starts on the same tick at each pass of a loop, after a seek and in an
//!   offline export; stopped, the value holds), `lfo_phase` the offset of both channels and
//!   `lfo_stereo` the offset of the right one, `lfo_depth` in octaves either side of the centre.
//!   In free mode `lfo_restart` sets the phase back to `lfo_phase` (and the random sequence to
//!   its start) when the transport starts playing; off, the LFO runs on freely (it also runs
//!   while the transport is stopped). [`Movement::reset`] sets it back too.
//! - The LFO's term goes through a slew limit of [`MOD_SLEW_OCTAVES_PER_SECOND`] (2 octaves
//!   per millisecond): the edges of the square and random shapes and the jumps of `lfo_phase`,
//!   `lfo_stereo`, `lfo_division` or `lfo_shape` become ramps of a few milliseconds instead of
//!   clicks, while a smooth shape in range is never touched (a 20 Hz sine at 5 octaves moves
//!   0.63 octave per millisecond). The limiter starts from the LFO's value, not from where it
//!   was, on the first frame after the block is built or reset and on each start of the
//!   transport, so every render from the same start is the same.
//! - **Envelope follower**: the input's level with `env_attack` and `env_release` time
//!   constants, times `env_amount` in octaves (positive: up as the sound gets louder). Both
//!   channels share it. It is smooth already (a one-pole) and not limited.
//!
//! `lfo_depth` and `env_amount` follow the contract's linear ramp over the frames the device
//! passes ([`Movement::set`]); the other settings change a rate or a shape, never a value, and
//! the slew limit smooths what they move. With a depth of 0 the LFO is not read at all.
//!
//! Plain state and arithmetic: no allocation, no lock, no panic on any input.

use super::envelope_follower::EnvelopeFollower;
use super::lfo::{self, DIVISION_KEYS, Lfo, SHAPE_KEYS, Shape};
use crate::device::{DeviceDescriptor, ParamDescriptor, Scale, Transport, Unit};
use crate::msg::ParamId;
use crate::param::Smoother;
use crate::time::TimeSignature;

/// Fastest move of the LFO's term, in octaves per second.
pub const MOD_SLEW_OCTAVES_PER_SECOND: f32 = 2000.0;
/// Deepest LFO and largest envelope amount, in octaves.
pub const DEPTH_MAX: f64 = 5.0;

const VALUES_SYNC: &[&str] = &["free", "synced"];
const VALUES_RESTART: &[&str] = &["off", "on"];

/// The ids a device gives the eleven movement parameters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ids {
    pub shape: u32,
    pub sync: u32,
    pub rate: u32,
    pub division: u32,
    pub phase: u32,
    pub stereo: u32,
    pub depth: u32,
    pub restart: u32,
    pub amount: u32,
    pub attack: u32,
    pub release: u32,
}

const fn choice(
    id: u32,
    key: &'static str,
    description: &'static str,
    default: f64,
    values: &'static [&'static str],
) -> ParamDescriptor {
    ParamDescriptor {
        id: ParamId(id),
        key,
        description,
        unit: Unit::None,
        min: 0.0,
        max: (values.len() - 1) as f64,
        default,
        scale: Scale::Linear,
        step: 1.0,
        smoothing_seconds: 0.0,
        values,
    }
}

#[allow(clippy::too_many_arguments)]
const fn number(
    id: u32,
    key: &'static str,
    description: &'static str,
    unit: Unit,
    (min, max, default): (f64, f64, f64),
    scale: Scale,
    smoothing_seconds: f64,
) -> ParamDescriptor {
    ParamDescriptor {
        id: ParamId(id),
        key,
        description,
        unit,
        min,
        max,
        default,
        scale,
        step: 0.0,
        smoothing_seconds,
        values: &[],
    }
}

/// `lfo_shape`: an index of [`SHAPE_KEYS`].
pub const fn lfo_shape(id: u32, description: &'static str, default: Shape) -> ParamDescriptor {
    choice(
        id,
        "lfo_shape",
        description,
        default as u32 as f64,
        SHAPE_KEYS,
    )
}

/// `lfo_sync`: free (0) or synced (1).
pub const fn lfo_sync(id: u32, description: &'static str, synced: bool) -> ParamDescriptor {
    choice(
        id,
        "lfo_sync",
        description,
        if synced { 1.0 } else { 0.0 },
        VALUES_SYNC,
    )
}

/// `lfo_rate`: 0.01 to 20 Hz, log.
pub const fn lfo_rate(id: u32, description: &'static str, default: f64) -> ParamDescriptor {
    number(
        id,
        "lfo_rate",
        description,
        Unit::Hertz,
        (0.01, 20.0, default),
        Scale::Log,
        0.0,
    )
}

/// `lfo_division`: an index of [`DIVISION_KEYS`].
pub const fn lfo_division(id: u32, description: &'static str, default: u32) -> ParamDescriptor {
    choice(
        id,
        "lfo_division",
        description,
        default as f64,
        &DIVISION_KEYS,
    )
}

/// `lfo_phase`: 0 to 360 degrees.
pub const fn lfo_phase(id: u32, description: &'static str) -> ParamDescriptor {
    number(
        id,
        "lfo_phase",
        description,
        Unit::Degrees,
        (0.0, 360.0, 0.0),
        Scale::Linear,
        0.0,
    )
}

/// `lfo_stereo`: 0 to 180 degrees.
pub const fn lfo_stereo(id: u32, description: &'static str) -> ParamDescriptor {
    number(
        id,
        "lfo_stereo",
        description,
        Unit::Degrees,
        (0.0, 180.0, 0.0),
        Scale::Linear,
        0.0,
    )
}

/// `lfo_depth`: 0 to [`DEPTH_MAX`] octaves, ramped over 20 ms.
pub const fn lfo_depth(id: u32, description: &'static str, default: f64) -> ParamDescriptor {
    number(
        id,
        "lfo_depth",
        description,
        Unit::Octaves,
        (0.0, DEPTH_MAX, default),
        Scale::Linear,
        0.02,
    )
}

/// `lfo_restart`: off (0) or on (1), default on.
pub const fn lfo_restart(id: u32, description: &'static str) -> ParamDescriptor {
    choice(id, "lfo_restart", description, 1.0, VALUES_RESTART)
}

/// `env_amount`: -[`DEPTH_MAX`] to +[`DEPTH_MAX`] octaves, default 0, ramped over 20 ms.
pub const fn env_amount(id: u32, description: &'static str) -> ParamDescriptor {
    number(
        id,
        "env_amount",
        description,
        Unit::Octaves,
        (-DEPTH_MAX, DEPTH_MAX, 0.0),
        Scale::Linear,
        0.02,
    )
}

/// `env_attack`: 1 ms to 500 ms, log, default 10 ms.
pub const fn env_attack(id: u32, description: &'static str) -> ParamDescriptor {
    number(
        id,
        "env_attack",
        description,
        Unit::Seconds,
        (0.001, 0.5, 0.01),
        Scale::Log,
        0.0,
    )
}

/// `env_release`: 10 ms to 2 s, log, default 150 ms.
pub const fn env_release(id: u32, description: &'static str) -> ParamDescriptor {
    number(
        id,
        "env_release",
        description,
        Unit::Seconds,
        (0.01, 2.0, 0.15),
        Scale::Log,
        0.0,
    )
}

/// The settings of the LFO that are not ramped.
#[derive(Clone, Copy, Debug)]
struct Settings {
    shape: Shape,
    synced: bool,
    rate_hz: f32,
    division: u32,
    /// `lfo_phase` and `lfo_stereo`, in cycles.
    phase: f64,
    stereo: f64,
    restart: bool,
}

/// The movement's state (module doc). It holds no more than the device held before the block
/// was shared (#688): the device passes its ids and its sample rate to each call.
#[derive(Clone, Copy, Debug)]
pub struct Movement {
    lfo: Lfo,
    settings: Settings,
    /// The time signature of the block (a division in bars).
    signature: TimeSignature,
    env: EnvelopeFollower,
    depth: Smoother,
    amount: Smoother,
    /// The LFO's term of each channel after the slew limit, in octaves.
    swing: [f32; 2],
    /// The limiter takes the LFO's value at once on the next frame (module doc).
    snap_swing: bool,
}

impl Movement {
    /// The movement at the defaults `descriptor` declares for `ids`, at `sample_rate` (a
    /// valid rate: the device has checked it).
    pub fn new(descriptor: &DeviceDescriptor, ids: &Ids, sample_rate: f32) -> Self {
        let default = |id: u32| descriptor.param(ParamId(id)).map_or(0.0, |d| d.default);
        Self {
            lfo: Lfo::new(),
            settings: Settings {
                shape: Shape::from_index(default(ids.shape) as u32),
                synced: default(ids.sync) >= 0.5,
                rate_hz: default(ids.rate) as f32,
                division: default(ids.division) as u32,
                phase: default(ids.phase) / 360.0,
                stereo: default(ids.stereo) / 360.0,
                restart: default(ids.restart) >= 0.5,
            },
            signature: TimeSignature::COMMON,
            env: EnvelopeFollower::new(
                default(ids.attack) as f32,
                default(ids.release) as f32,
                sample_rate,
            ),
            depth: Smoother::new(default(ids.depth) as f32),
            amount: Smoother::new(default(ids.amount) as f32),
            swing: [0.0; 2],
            snap_swing: true,
        }
    }

    /// The start of a block (or of a piece of one) at `transport`, at `sample_rate`: re-anchors
    /// the synced position, restarts a free LFO with `lfo_restart` when the transport starts
    /// playing, and takes the block's time signature.
    pub fn begin(&mut self, transport: &Transport, sample_rate: f32) {
        let s = self.settings;
        if self
            .lfo
            .begin(transport, sample_rate, s.restart && !s.synced)
        {
            self.snap_swing = true;
        }
        self.signature = transport.signature;
    }

    /// Sets parameter `id` (one of `ids`) to `value` (sanitized by the descriptor), the ramped
    /// ones over `ramp` frames, at `sample_rate`. Returns whether `id` is one of the movement's.
    pub fn set(&mut self, ids: &Ids, id: u32, value: f32, ramp: u32, sample_rate: f32) -> bool {
        let s = &mut self.settings;
        if id == ids.shape {
            s.shape = Shape::from_index(value.round() as u32);
        } else if id == ids.sync {
            s.synced = value >= 0.5;
        } else if id == ids.rate {
            s.rate_hz = value;
        } else if id == ids.division {
            s.division = value.round() as u32;
        } else if id == ids.phase {
            s.phase = f64::from(value) / 360.0;
        } else if id == ids.stereo {
            s.stereo = f64::from(value) / 360.0;
        } else if id == ids.depth {
            self.depth.set(value, ramp);
        } else if id == ids.restart {
            s.restart = value >= 0.5;
        } else if id == ids.amount {
            self.amount.set(value, ramp);
        } else if id == ids.attack {
            self.env.set_attack(value, sample_rate);
        } else if id == ids.release {
            self.env.set_release(value, sample_rate);
        } else {
            return false;
        }
        true
    }

    /// One frame at `sample_rate`: the modulation of the left and the right channel, in
    /// octaves, from the input's rectified `level`.
    #[inline]
    pub fn next(&mut self, level: f32, sample_rate: f32) -> [f32; 2] {
        let s = self.settings;
        let depth = self.depth.next_value();
        let amount = self.amount.next_value();
        let env = self.env.next(level);
        let target = if depth > 0.0 {
            let period = lfo::division_ticks(s.division, self.signature);
            let base = self.lfo.phase(s.synced, period) + s.phase;
            [
                lfo::value(s.shape, base) * depth,
                lfo::value(s.shape, base + s.stereo) * depth,
            ]
        } else {
            [0.0; 2]
        };
        if self.snap_swing {
            self.swing = target;
            self.snap_swing = false;
        } else {
            let slew = MOD_SLEW_OCTAVES_PER_SECOND / sample_rate;
            for (swing, t) in self.swing.iter_mut().zip(target) {
                *swing += (t - *swing).clamp(-slew, slew);
            }
        }
        self.lfo.advance(s.rate_hz, sample_rate);
        let e = env * amount;
        [self.swing[0] + e, self.swing[1] + e]
    }

    /// The envelope follower's level after the last frame, in `[0, 1]`.
    pub fn envelope(&self) -> f32 {
        self.env.level()
    }

    /// Ends the ramps of the depth and the amount at their targets.
    pub fn snap(&mut self) {
        self.depth.snap();
        self.amount.snap();
    }

    /// Sets the LFO and the envelope back to their start and snaps the ramps; the settings
    /// are kept.
    pub fn reset(&mut self) {
        self.snap();
        self.lfo.reset();
        self.env.reset();
        self.swing = [0.0; 2];
        self.snap_swing = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::DeviceRole;
    use crate::msg::DeviceKind;
    use crate::time::Tempo;

    const IDS: Ids = Ids {
        shape: 0,
        sync: 1,
        rate: 2,
        division: 3,
        phase: 4,
        stereo: 5,
        depth: 6,
        restart: 7,
        amount: 8,
        attack: 9,
        release: 10,
    };

    const D: DeviceDescriptor = DeviceDescriptor {
        kind: "test.movement",
        kind_id: DeviceKind(0),
        version: 1,
        name: "Movement",
        role: DeviceRole::Effect,
        memory_bytes: 0,
        midi_output: None,
        named_keys: &[],
        params: &[
            lfo_shape(0, "", Shape::Square),
            lfo_sync(1, "", false),
            lfo_rate(2, "", 1.0),
            lfo_division(3, "", 6),
            lfo_phase(4, ""),
            lfo_stereo(5, ""),
            lfo_depth(6, "", 1.0),
            lfo_restart(7, ""),
            env_amount(8, ""),
            env_attack(9, ""),
            env_release(10, ""),
        ],
    };
    const SR: f32 = 48_000.0;

    fn playing(position: f64) -> Transport {
        Transport {
            playing: true,
            tempo: Tempo::new(120.0).unwrap(),
            position_ticks: position,
            ..Transport::default()
        }
    }

    #[test]
    fn the_builders_hold_the_shared_ranges_and_the_given_defaults() {
        for p in D.params {
            assert!(p.min <= p.default && p.default <= p.max, "{}", p.key);
            if !p.values.is_empty() {
                assert_eq!(p.max, (p.values.len() - 1) as f64, "{}", p.key);
            }
        }
        assert_eq!(D.params[0].default, 4.0);
        assert_eq!(D.params[0].values, SHAPE_KEYS);
        assert_eq!(D.params[3].values.len(), 13);
        assert_eq!(D.params[6].max, DEPTH_MAX);
        assert_eq!(D.params[8].min, -DEPTH_MAX);
    }

    #[test]
    fn the_limiter_snaps_on_the_first_frame_then_ramps_a_square_edge() {
        let mut m = Movement::new(&D, &IDS, SR);
        m.begin(&playing(0.0), SR);
        // A free square at 1 Hz, depth 1: +1 octave at once on the first frame.
        assert_eq!(m.next(0.0, SR), [1.0, 1.0]);
        // Half a cycle later the target is -1: the limiter takes 1 ms per 2 octaves.
        for _ in 1..24_000 {
            m.next(0.0, SR);
        }
        let after = m.next(0.0, SR);
        let step = MOD_SLEW_OCTAVES_PER_SECOND / SR;
        assert!((after[0] - (1.0 - step)).abs() < 1e-5, "{after:?}");
        for _ in 0..60 {
            m.next(0.0, SR);
        }
        assert_eq!(m.next(0.0, SR)[0], -1.0);
        // A reset snaps again.
        m.reset();
        m.begin(&playing(0.0), SR);
        assert_eq!(m.next(0.0, SR), [1.0, 1.0]);
    }

    #[test]
    fn restart_applies_to_a_free_lfo_when_play_starts_and_never_to_a_synced_one() {
        let stopped = Transport {
            playing: false,
            ..playing(0.0)
        };
        let mut m = Movement::new(&D, &IDS, SR);
        m.set(&IDS, IDS.shape, 0.0, 0, SR); // sine
        m.begin(&stopped, SR);
        let first = m.next(0.0, SR);
        for _ in 0..1000 {
            m.next(0.0, SR);
        }
        // Play starts: the free sine is back at its phase 0, snapped.
        m.begin(&playing(0.0), SR);
        assert_eq!(m.next(0.0, SR), first);
        // Synced: the position decides, restart does not.
        let mut s = Movement::new(&D, &IDS, SR);
        s.set(&IDS, IDS.sync, 1.0, 0, SR);
        s.set(&IDS, IDS.shape, 0.0, 0, SR);
        s.begin(&playing(240.0), SR); // an eighth into a quarter cycle: sine at 90 degrees
        assert!((s.next(0.0, SR)[0] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn depth_zero_reads_no_lfo_and_the_envelope_adds_its_amount() {
        let mut m = Movement::new(&D, &IDS, SR);
        assert!(m.set(&IDS, IDS.depth, 0.0, 0, SR));
        assert!(m.set(&IDS, IDS.amount, 2.0, 0, SR));
        assert!(m.set(&IDS, IDS.attack, 0.001, 0, SR));
        assert!(!m.set(&IDS, 99, 1.0, 0, SR));
        m.begin(&playing(0.0), SR);
        let mut last = [0.0; 2];
        for _ in 0..4800 {
            last = m.next(1.0, SR);
        }
        assert!((last[0] - 2.0).abs() < 1e-4, "{last:?}");
        assert_eq!(last[0], last[1]);
        assert!((m.envelope() - 1.0).abs() < 1e-4);
    }
}
