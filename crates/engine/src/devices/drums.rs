//! The synthesized drums, `faulk.drums`, displayed "Drums" (DV-6): four sounds, each made by
//! its own small synthesizer, mapped on the notes of a General MIDI drum map. A [`Device`] of
//! role instrument, with no sample: every sound is computed.
//!
//! # Notes
//!
//! | Key | Note | Sound |
//! |---|---|---|
//! | 36 | C2 | kick: a sine whose pitch falls fast, with an exponential decay |
//! | 38 | D2 | snare: a short tone and highpassed noise |
//! | 39 | D#2 | clap: three short bursts of bandpassed noise, then a tail |
//! | 42 | F#2 | closed hi-hat: six square oscillators and noise, highpassed, a short decay |
//! | 46 | A#2 | open hi-hat: the same sound with its own, longer decay |
//!
//! The closed and the open hi-hat are one sound (as on a real kit, a closed hit chokes an
//! open one). Any other key is ignored. The velocity (0 to 1) scales the hit; a velocity of
//! 0 or not finite is no hit. A drum hit is a one-shot: a `NoteOff` is ignored and the sound
//! rings out; `AllNotesOff` fades every sound out in 3 ms. The fine tuning of a note
//! (`tune_cents`) is ignored: the pitch of a drum is its parameter.
//!
//! A sound hit again while it still rings fades out in 3 ms (no click), then starts again.
//! A hit's noise comes from a generator that starts at the same state after [`Device::reset`]
//! and at construction, so a render is the same every time (native and WASM alike).
//!
//! # Parameters and smoothing
//!
//! The parameters of each sound are in the descriptor, named `<sound>_<what>` (`kick_tune`,
//! `snare_decay`, `clap_tone`, `hat_open_decay`…), then the master `gain`. Every value goes
//! through [`ParamDescriptor::sanitize`]. A decay is the time for the sound to fall by 60 dB.
//! The tuning, decay, punch, snap and tone of a sound are read when a hit starts (they apply
//! to the next hit, so they are never smoothed); **the level of each sound and the master
//! gain follow the 10 ms linear ramp the descriptor declares**.

use core::f32::consts::TAU;

use super::Builtin;
use super::osc_synth::LinearRamp;
use crate::device::{
    AudioIo, BlockCtx, Device, DeviceDescriptor, DeviceRole, DeviceSetup, Event, ParamDescriptor,
    Scale, Unit, event_frame,
};
use crate::msg::{DeviceKind, MsgError, ParamId, try_box};

/// Key of the kick (C2).
pub const KICK_KEY: u8 = 36;
/// Key of the snare (D2).
pub const SNARE_KEY: u8 = 38;
/// Key of the clap (D#2).
pub const CLAP_KEY: u8 = 39;
/// Key of the closed hi-hat (F#2).
pub const CLOSED_HAT_KEY: u8 = 42;
/// Key of the open hi-hat (A#2).
pub const OPEN_HAT_KEY: u8 = 46;

/// Ids of the parameters (the index of the value in the device, and the `ParamId` number).
pub const KICK_TUNE: u32 = 0;
pub const KICK_DECAY: u32 = 1;
pub const KICK_PUNCH: u32 = 2;
pub const KICK_LEVEL: u32 = 3;
pub const SNARE_TUNE: u32 = 4;
pub const SNARE_DECAY: u32 = 5;
pub const SNARE_SNAP: u32 = 6;
pub const SNARE_LEVEL: u32 = 7;
pub const CLAP_TONE: u32 = 8;
pub const CLAP_DECAY: u32 = 9;
pub const CLAP_LEVEL: u32 = 10;
pub const HAT_TONE: u32 = 11;
pub const HAT_DECAY: u32 = 12;
pub const HAT_OPEN_DECAY: u32 = 13;
pub const HAT_LEVEL: u32 = 14;
pub const GAIN: u32 = 15;

const PARAM_COUNT: usize = 16;
/// The parameters that are levels, smoothed, in the order of [`Drums::levels`]: the four
/// sounds, then the master gain.
const LEVEL_IDS: [u32; 5] = [KICK_LEVEL, SNARE_LEVEL, CLAP_LEVEL, HAT_LEVEL, GAIN];

/// Duration of the fade of a sound hit again or silenced, in seconds.
const FADE_SECONDS: f32 = 0.003;
/// A sound whose envelope falls below this (about -80 dB) has ended.
const END_LEVEL: f32 = 1e-4;
/// Samples whose magnitude is below this are flushed to zero (denormal guard).
const DENORMAL_FLOOR: f32 = 1e-20;
/// Time constant of the pitch fall of the kick and of the snare's tone, as a 60 dB time.
const KICK_SWEEP_SECONDS: f32 = 0.15;
const SNARE_SWEEP_SECONDS: f32 = 0.08;
/// The clap's bursts: start of each, in seconds, and the 60 dB time of a burst.
const CLAP_BURSTS: [f32; 4] = [0.0, 0.011, 0.022, 0.033];
const CLAP_BURST_SECONDS: f32 = 0.02;
/// Frequencies of the six oscillators of the hi-hat at a tone of [`HAT_BASE_TONE`], in hertz.
const HAT_OSCILLATORS: [f32; 6] = [205.3, 304.4, 369.6, 522.7, 540.0, 800.0];
const HAT_BASE_TONE: f32 = 2_000.0;

const fn level(id: u32, key: &'static str, default: f64) -> ParamDescriptor {
    ParamDescriptor {
        id: ParamId(id),
        key,
        unit: Unit::Gain,
        min: 0.0,
        max: 1.0,
        default,
        scale: Scale::Linear,
        step: 0.0,
        smoothing_seconds: 0.01,
        values: &[],
    }
}

/// A parameter read when a hit starts: never smoothed.
const fn at_hit(
    id: u32,
    key: &'static str,
    unit: Unit,
    scale: Scale,
    min: f64,
    max: f64,
    default: f64,
) -> ParamDescriptor {
    ParamDescriptor {
        id: ParamId(id),
        key,
        unit,
        min,
        max,
        default,
        scale,
        step: 0.0,
        smoothing_seconds: 0.0,
        values: &[],
    }
}

/// The descriptor of the drums (CT-5): `faulk.drums`, displayed "Drums" (one English name,
/// never translated). The defaults are the values of [`Drums::new`].
pub const DESCRIPTOR: DeviceDescriptor = DeviceDescriptor {
    kind: "faulk.drums",
    kind_id: DeviceKind(2),
    version: 1,
    name: "Drums",
    role: DeviceRole::Instrument,
    memory_bytes: size_of::<Drums>() as u64,
    midi_output: None,
    params: &[
        at_hit(
            KICK_TUNE,
            "kick_tune",
            Unit::Hertz,
            Scale::Log,
            30.0,
            120.0,
            52.0,
        ),
        at_hit(
            KICK_DECAY,
            "kick_decay",
            Unit::Seconds,
            Scale::Log,
            0.05,
            1.5,
            0.45,
        ),
        at_hit(
            KICK_PUNCH,
            "kick_punch",
            Unit::None,
            Scale::Linear,
            0.0,
            1.0,
            0.6,
        ),
        level(KICK_LEVEL, "kick_level", 0.9),
        at_hit(
            SNARE_TUNE,
            "snare_tune",
            Unit::Hertz,
            Scale::Log,
            100.0,
            400.0,
            185.0,
        ),
        at_hit(
            SNARE_DECAY,
            "snare_decay",
            Unit::Seconds,
            Scale::Log,
            0.05,
            0.8,
            0.2,
        ),
        at_hit(
            SNARE_SNAP,
            "snare_snap",
            Unit::None,
            Scale::Linear,
            0.0,
            1.0,
            0.6,
        ),
        level(SNARE_LEVEL, "snare_level", 0.8),
        at_hit(
            CLAP_TONE,
            "clap_tone",
            Unit::Hertz,
            Scale::Log,
            600.0,
            4_000.0,
            1_500.0,
        ),
        at_hit(
            CLAP_DECAY,
            "clap_decay",
            Unit::Seconds,
            Scale::Log,
            0.05,
            1.0,
            0.3,
        ),
        level(CLAP_LEVEL, "clap_level", 0.7),
        at_hit(
            HAT_TONE,
            "hat_tone",
            Unit::Hertz,
            Scale::Log,
            3_000.0,
            12_000.0,
            7_000.0,
        ),
        at_hit(
            HAT_DECAY,
            "hat_decay",
            Unit::Seconds,
            Scale::Log,
            0.01,
            0.4,
            0.05,
        ),
        at_hit(
            HAT_OPEN_DECAY,
            "hat_open_decay",
            Unit::Seconds,
            Scale::Log,
            0.1,
            2.0,
            0.5,
        ),
        level(HAT_LEVEL, "hat_level", 0.6),
        level(GAIN, "gain", 0.8),
    ],
};

/// The drums' line of the registry ([`super::REGISTRY`]).
pub const BUILTIN: Builtin = Builtin {
    descriptor: &DESCRIPTOR,
    factory,
};

/// Builds the drums for `setup`, off the audio path (fallible allocation).
pub fn factory(setup: &DeviceSetup) -> Result<Box<dyn Device>, MsgError> {
    Ok(try_box(Drums::new(setup.sample_rate))?)
}

// ---------------------------------------------------------------- pieces

/// Per-sample factor that makes an exponential fall by 60 dB in `seconds`.
fn decay_coef(seconds: f32, sample_rate: f32) -> f32 {
    (-6.907_755 / (seconds.max(1e-4) * sample_rate)).exp()
}

/// A white noise generator (xorshift32): integer arithmetic, so the same in every build.
#[derive(Clone, Copy, Debug)]
struct Noise(u32);

impl Noise {
    fn next(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        // The top 24 bits as a number in [-1, 1).
        (x >> 8) as f32 / 8_388_608.0 - 1.0
    }
}

/// A state variable filter (zero-delay form): stable for any cutoff below Nyquist.
#[derive(Clone, Copy, Debug, Default)]
struct Svf {
    a1: f32,
    a2: f32,
    a3: f32,
    k: f32,
    ic1: f32,
    ic2: f32,
}

impl Svf {
    fn tune(&mut self, cutoff: f32, q: f32, sample_rate: f32) {
        let cutoff = cutoff.clamp(20.0, sample_rate * 0.45);
        let g = (core::f32::consts::PI * cutoff / sample_rate).tan();
        self.k = 1.0 / q;
        self.a1 = 1.0 / (1.0 + g * (g + self.k));
        self.a2 = g * self.a1;
        self.a3 = g * self.a2;
        self.ic1 = 0.0;
        self.ic2 = 0.0;
    }

    /// The band (peak gain 1 at the cutoff) and the high output of the next sample.
    fn process(&mut self, x: f32) -> (f32, f32) {
        let v3 = x - self.ic2;
        let v1 = self.a1 * self.ic1 + self.a2 * v3;
        let v2 = self.ic2 + self.a2 * self.ic1 + self.a3 * v3;
        self.ic1 = flush(2.0 * v1 - self.ic1);
        self.ic2 = flush(2.0 * v2 - self.ic2);
        (self.k * v1, x - self.k * v1 - v2)
    }
}

fn flush(x: f32) -> f32 {
    if x.abs() < DENORMAL_FLOOR { 0.0 } else { x }
}

// ---------------------------------------------------------------- the sounds

#[derive(Clone, Copy, Debug)]
struct Kick {
    active: bool,
    amplitude: f32,
    phase: f32,
    /// Phase increment at the tuned pitch, and the extra pitch (a factor) at the hit.
    inc: f32,
    punch: f32,
    sweep: f32,
    sweep_coef: f32,
    env: f32,
    env_coef: f32,
}

impl Kick {
    const IDLE: Self = Self {
        active: false,
        amplitude: 0.0,
        phase: 0.0,
        inc: 0.0,
        punch: 0.0,
        sweep: 0.0,
        sweep_coef: 0.0,
        env: 0.0,
        env_coef: 0.0,
    };

    fn start(&mut self, velocity: f32, p: &Values, sample_rate: f32) {
        *self = Self {
            active: true,
            amplitude: velocity,
            phase: 0.0,
            inc: (p.get(KICK_TUNE) / sample_rate).min(0.5),
            punch: 4.0 * p.get(KICK_PUNCH),
            sweep: 1.0,
            sweep_coef: decay_coef(KICK_SWEEP_SECONDS, sample_rate),
            env: 1.0,
            env_coef: decay_coef(p.get(KICK_DECAY), sample_rate),
        };
    }

    fn next(&mut self) -> f32 {
        let inc = (self.inc * (1.0 + self.punch * self.sweep)).min(0.5);
        self.phase += inc;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        let out = (self.phase * TAU).sin() * self.env * self.amplitude;
        self.sweep *= self.sweep_coef;
        self.env *= self.env_coef;
        if self.env < END_LEVEL {
            self.active = false;
        }
        out
    }
}

#[derive(Clone, Copy, Debug)]
struct Snare {
    active: bool,
    amplitude: f32,
    snap: f32,
    noise: Noise,
    filter: Svf,
    phase: f32,
    inc: f32,
    sweep: f32,
    sweep_coef: f32,
    body_env: f32,
    body_coef: f32,
    noise_env: f32,
    noise_coef: f32,
}

impl Snare {
    const SEED: u32 = 0x9E37_79B9;

    const IDLE: Self = Self {
        active: false,
        amplitude: 0.0,
        snap: 0.0,
        noise: Noise(Self::SEED),
        filter: Svf {
            a1: 0.0,
            a2: 0.0,
            a3: 0.0,
            k: 0.0,
            ic1: 0.0,
            ic2: 0.0,
        },
        phase: 0.0,
        inc: 0.0,
        sweep: 0.0,
        sweep_coef: 0.0,
        body_env: 0.0,
        body_coef: 0.0,
        noise_env: 0.0,
        noise_coef: 0.0,
    };

    fn start(&mut self, velocity: f32, p: &Values, sample_rate: f32) {
        let decay = p.get(SNARE_DECAY);
        let noise = self.noise;
        let mut filter = Svf::default();
        filter.tune(1_800.0, 0.7, sample_rate);
        *self = Self {
            active: true,
            amplitude: velocity,
            snap: p.get(SNARE_SNAP),
            noise,
            filter,
            phase: 0.0,
            inc: (p.get(SNARE_TUNE) / sample_rate).min(0.5),
            sweep: 1.0,
            sweep_coef: decay_coef(SNARE_SWEEP_SECONDS, sample_rate),
            body_env: 1.0,
            body_coef: decay_coef(decay * 0.6, sample_rate),
            noise_env: 1.0,
            noise_coef: decay_coef(decay, sample_rate),
        };
    }

    fn next(&mut self) -> f32 {
        let inc = (self.inc * (1.0 + 0.3 * self.sweep)).min(0.5);
        self.phase += inc;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        let body = (self.phase * TAU).sin() * self.body_env * 0.7 * (1.0 - 0.4 * self.snap);
        let (_, high) = self.filter.process(self.noise.next());
        let hiss = high * self.noise_env * 0.8 * self.snap;
        self.sweep *= self.sweep_coef;
        self.body_env *= self.body_coef;
        self.noise_env *= self.noise_coef;
        if self.noise_env < END_LEVEL {
            self.active = false;
        }
        (body + hiss) * self.amplitude
    }
}

#[derive(Clone, Copy, Debug)]
struct Clap {
    active: bool,
    amplitude: f32,
    noise: Noise,
    filter: Svf,
    /// Frames since the hit, and the index of the next burst to start.
    age: u32,
    next_burst: usize,
    burst_frames: [u32; 4],
    env: f32,
    burst_coef: f32,
    tail_coef: f32,
    coef: f32,
}

impl Clap {
    const SEED: u32 = 0x85EB_CA6B;

    const IDLE: Self = Self {
        active: false,
        amplitude: 0.0,
        noise: Noise(Self::SEED),
        filter: Svf {
            a1: 0.0,
            a2: 0.0,
            a3: 0.0,
            k: 0.0,
            ic1: 0.0,
            ic2: 0.0,
        },
        age: 0,
        next_burst: 0,
        burst_frames: [0; 4],
        env: 0.0,
        burst_coef: 0.0,
        tail_coef: 0.0,
        coef: 0.0,
    };

    fn start(&mut self, velocity: f32, p: &Values, sample_rate: f32) {
        let noise = self.noise;
        let mut filter = Svf::default();
        filter.tune(p.get(CLAP_TONE), 1.5, sample_rate);
        let mut burst_frames = [0; 4];
        for (frame, seconds) in burst_frames.iter_mut().zip(CLAP_BURSTS) {
            *frame = (seconds * sample_rate) as u32;
        }
        *self = Self {
            active: true,
            amplitude: velocity,
            noise,
            filter,
            age: 0,
            next_burst: 0,
            burst_frames,
            env: 0.0,
            burst_coef: decay_coef(CLAP_BURST_SECONDS, sample_rate),
            tail_coef: decay_coef(p.get(CLAP_DECAY), sample_rate),
            coef: 0.0,
        };
    }

    fn next(&mut self) -> f32 {
        // Three short bursts, then the last one is the tail, which decays at the set time.
        if self.next_burst < CLAP_BURSTS.len() && self.age >= self.burst_frames[self.next_burst] {
            self.env = 1.0;
            self.coef = if self.next_burst + 1 < CLAP_BURSTS.len() {
                self.burst_coef
            } else {
                self.tail_coef
            };
            self.next_burst += 1;
        }
        let (band, _) = self.filter.process(self.noise.next());
        let out = band * 2.5 * self.env * self.amplitude;
        self.env *= self.coef;
        self.age = self.age.saturating_add(1);
        if self.next_burst >= CLAP_BURSTS.len() && self.env < END_LEVEL {
            self.active = false;
        }
        out
    }
}

#[derive(Clone, Copy, Debug)]
struct Hat {
    active: bool,
    amplitude: f32,
    noise: Noise,
    filter: Svf,
    phases: [f32; 6],
    incs: [f32; 6],
    env: f32,
    env_coef: f32,
}

impl Hat {
    const SEED: u32 = 0xC2B2_AE35;

    const IDLE: Self = Self {
        active: false,
        amplitude: 0.0,
        noise: Noise(Self::SEED),
        filter: Svf {
            a1: 0.0,
            a2: 0.0,
            a3: 0.0,
            k: 0.0,
            ic1: 0.0,
            ic2: 0.0,
        },
        phases: [0.0; 6],
        incs: [0.0; 6],
        env: 0.0,
        env_coef: 0.0,
    };

    fn start(&mut self, velocity: f32, open: bool, p: &Values, sample_rate: f32) {
        let noise = self.noise;
        let tone = p.get(HAT_TONE);
        let mut filter = Svf::default();
        filter.tune(tone, 0.8, sample_rate);
        let mut incs = [0.0; 6];
        for (inc, freq) in incs.iter_mut().zip(HAT_OSCILLATORS) {
            *inc = (freq * tone / HAT_BASE_TONE / sample_rate).min(0.45);
        }
        let decay = p.get(if open { HAT_OPEN_DECAY } else { HAT_DECAY });
        *self = Self {
            active: true,
            amplitude: velocity,
            noise,
            filter,
            phases: [0.0; 6],
            incs,
            env: 1.0,
            env_coef: decay_coef(decay, sample_rate),
        };
    }

    fn next(&mut self) -> f32 {
        let mut metal = 0.0;
        for (phase, inc) in self.phases.iter_mut().zip(self.incs) {
            *phase += inc;
            if *phase >= 1.0 {
                *phase -= 1.0;
            }
            metal += if *phase < 0.5 { 1.0 } else { -1.0 };
        }
        let source = metal / 6.0 * 0.5 + self.noise.next() * 0.5;
        let (_, high) = self.filter.process(source);
        let out = high * 1.5 * self.env * self.amplitude;
        self.env *= self.env_coef;
        if self.env < END_LEVEL {
            self.active = false;
        }
        out
    }
}

// ---------------------------------------------------------------- the device

/// The values of the parameters, indexed by id.
#[derive(Clone, Copy, Debug)]
struct Values([f32; PARAM_COUNT]);

impl Values {
    fn get(&self, id: u32) -> f32 {
        self.0.get(id as usize).copied().unwrap_or(0.0)
    }
}

/// A hit waiting for its sound to fade out.
#[derive(Clone, Copy, Debug)]
struct Hit {
    velocity: f32,
    open: bool,
}

/// The state of a sound's slot: a hit while it rings, or a silencing, fades it out first.
#[derive(Clone, Copy, Debug)]
struct Slot {
    fading: bool,
    /// 1 at full, 0 when the fade is over.
    fade: f32,
    pending: Option<Hit>,
}

impl Slot {
    const IDLE: Self = Self {
        fading: false,
        fade: 1.0,
        pending: None,
    };
}

const KICK: usize = 0;
const SNARE: usize = 1;
const CLAP: usize = 2;
const HAT: usize = 3;
const SOUNDS: usize = 4;

/// The synthesized drums. All memory is allocated in [`Drums::new`].
#[derive(Clone, Debug)]
pub struct Drums {
    sample_rate: f32,
    values: Values,
    /// The four sounds' levels, then the master gain.
    levels: [LinearRamp; 5],
    level_frames: [u32; 5],
    kick: Kick,
    snare: Snare,
    clap: Clap,
    hat: Hat,
    slots: [Slot; SOUNDS],
    fade_step: f32,
}

impl Drums {
    pub fn new(sample_rate: f32) -> Self {
        let sample_rate = if sample_rate.is_finite() && sample_rate >= 8_000.0 {
            sample_rate
        } else {
            48_000.0
        };
        let mut values = [0.0; PARAM_COUNT];
        for p in DESCRIPTOR.params {
            values[p.id.0 as usize] = p.default as f32;
        }
        let level_of = |i: usize| LEVEL_IDS.get(i).map_or(0.0, |id| values[*id as usize]);
        let frames_of = |i: usize| {
            DESCRIPTOR
                .param(ParamId(LEVEL_IDS[i]))
                .map_or(0, |d| d.smoothing_frames(sample_rate))
        };
        Self {
            sample_rate,
            values: Values(values),
            levels: core::array::from_fn(|i| LinearRamp::new(level_of(i))),
            level_frames: core::array::from_fn(frames_of),
            kick: Kick::IDLE,
            snare: Snare::IDLE,
            clap: Clap::IDLE,
            hat: Hat::IDLE,
            slots: [Slot::IDLE; SOUNDS],
            fade_step: 1.0 / (FADE_SECONDS * sample_rate),
        }
    }

    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    /// Current value of a parameter (the target of a smoothed one).
    pub fn param(&self, id: u32) -> f32 {
        self.values.get(id)
    }

    /// Number of sounds currently producing sound.
    pub fn active_voices(&self) -> usize {
        (0..SOUNDS).filter(|i| self.is_active(*i)).count()
    }

    fn is_active(&self, sound: usize) -> bool {
        match sound {
            KICK => self.kick.active,
            SNARE => self.snare.active,
            CLAP => self.clap.active,
            _ => self.hat.active,
        }
    }

    /// The sound a key plays, and whether it is the open hi-hat.
    fn sound_of(key: u8) -> Option<(usize, bool)> {
        match key {
            KICK_KEY => Some((KICK, false)),
            SNARE_KEY => Some((SNARE, false)),
            CLAP_KEY => Some((CLAP, false)),
            CLOSED_HAT_KEY => Some((HAT, false)),
            OPEN_HAT_KEY => Some((HAT, true)),
            _ => None,
        }
    }

    fn start_sound(&mut self, sound: usize, hit: Hit) {
        let sr = self.sample_rate;
        match sound {
            KICK => self.kick.start(hit.velocity, &self.values, sr),
            SNARE => self.snare.start(hit.velocity, &self.values, sr),
            CLAP => self.clap.start(hit.velocity, &self.values, sr),
            _ => self.hat.start(hit.velocity, hit.open, &self.values, sr),
        }
    }

    fn stop_sound(&mut self, sound: usize) {
        match sound {
            KICK => self.kick.active = false,
            SNARE => self.snare.active = false,
            CLAP => self.clap.active = false,
            _ => self.hat.active = false,
        }
    }

    /// A hit of `key`: starts its sound now, or after a short fade when it still rings. Keys
    /// the drums do not play, and a velocity of 0 or not finite, are ignored.
    fn hit(&mut self, key: u8, velocity: f32) {
        let Some((sound, open)) = Self::sound_of(key) else {
            return;
        };
        if !(velocity.is_finite() && velocity > 0.0) {
            return;
        }
        let hit = Hit {
            velocity: velocity.min(1.0),
            open,
        };
        if self.is_active(sound) {
            self.slots[sound].pending = Some(hit);
            self.slots[sound].fading = true;
        } else {
            self.slots[sound] = Slot::IDLE;
            self.start_sound(sound, hit);
        }
    }

    /// Fades every sound out and drops the hits waiting.
    fn silence(&mut self) {
        for sound in 0..SOUNDS {
            self.slots[sound].pending = None;
            if self.is_active(sound) {
                self.slots[sound].fading = true;
            }
        }
    }

    /// The next sample of a sound, through its fade.
    fn tick(&mut self, sound: usize) -> f32 {
        if self.slots[sound].fading {
            self.slots[sound].fade -= self.fade_step;
            if self.slots[sound].fade <= 0.0 {
                let pending = self.slots[sound].pending.take();
                self.slots[sound] = Slot::IDLE;
                self.stop_sound(sound);
                if let Some(hit) = pending {
                    self.start_sound(sound, hit);
                }
            }
        }
        if !self.is_active(sound) {
            return 0.0;
        }
        let raw = match sound {
            KICK => self.kick.next(),
            SNARE => self.snare.next(),
            CLAP => self.clap.next(),
            _ => self.hat.next(),
        };
        raw * self.slots[sound].fade
    }

    fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        let frames = left.len().min(right.len());
        for frame in 0..frames {
            let levels: [f32; 5] = core::array::from_fn(|i| self.levels[i].next_value());
            let mut sum = 0.0;
            for (sound, level) in levels.iter().take(SOUNDS).enumerate() {
                sum += self.tick(sound) * level;
            }
            let mut sample = (sum * levels[4]).tanh();
            if !sample.is_finite() || sample.abs() < DENORMAL_FLOOR {
                sample = 0.0;
            }
            left[frame] = sample;
            right[frame] = sample;
        }
    }

    /// Applies a parameter value: unknown ids and non-finite values are ignored, the rest is
    /// clamped by the descriptor; a level starts its ramp.
    fn set_value(&mut self, id: ParamId, value: f32) {
        let Some(desc) = DESCRIPTOR.param(id) else {
            return;
        };
        let Some(value) = desc.sanitize(value) else {
            return;
        };
        let Some(slot) = self.values.0.get_mut(id.0 as usize) else {
            return;
        };
        *slot = value;
        if let Some(i) = LEVEL_IDS.iter().position(|l| *l == id.0) {
            self.levels[i].set(value, self.level_frames[i]);
        }
    }

    fn apply(&mut self, event: &Event) {
        match *event {
            Event::NoteOn { key, velocity, .. } => self.hit(key, velocity),
            Event::NoteOff { .. } => {}
            Event::AllNotesOff { .. } => self.silence(),
            Event::ParamValue { id, value, .. } => self.set_value(id, value),
        }
    }
}

impl Device for Drums {
    fn descriptor(&self) -> &'static DeviceDescriptor {
        &DESCRIPTOR
    }

    /// Renders the frames between events and applies each event at its frame
    /// ([`event_frame`]).
    fn process(&mut self, ctx: &BlockCtx, events: &[Event], io: &mut AudioIo<'_>) {
        let frames = (ctx.frames as usize).min(io.left.len()).min(io.right.len());
        let mut cursor = 0;
        for event in events {
            let at = event_frame(event.offset(), cursor, frames);
            if at > cursor {
                self.render(&mut io.left[cursor..at], &mut io.right[cursor..at]);
                cursor = at;
            }
            self.apply(event);
        }
        if frames > cursor {
            self.render(&mut io.left[cursor..frames], &mut io.right[cursor..frames]);
        }
        crate::host::add_block_voices(self.active_voices() as u32);
    }

    fn set_param(&mut self, id: ParamId, value: f32) {
        self.set_value(id, value);
    }

    /// Silences every sound at once, restarts the noise and snaps the levels to their
    /// targets; the parameter values are kept.
    fn reset(&mut self) {
        self.kick = Kick::IDLE;
        self.snare = Snare::IDLE;
        self.clap = Clap::IDLE;
        self.hat = Hat::IDLE;
        self.slots = [Slot::IDLE; SOUNDS];
        for level in &mut self.levels {
            level.snap();
        }
    }
}
