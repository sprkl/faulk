//! One voice: three band-limited oscillators (A, B, a square sub one octave under A), the
//! input drive, the filter (ladder or state-variable), the output stage, two analog-style
//! envelopes and a filter "snap", plus the imperfections: slow drift of each oscillator and
//! fixed tolerances of the voice slot (tuning, cutoff, envelope times), all drawn from a
//! seeded random source so that renders are reproducible.
//!
//! Pitch, drift, cutoff and the filter envelope run at control rate (every [`CTRL`]
//! samples at the voice's rate); the filter coefficient is interpolated per sample between
//! control points.

use crate::env::{Adsr, Env};
use crate::filter::{LADDER_MAX_K, Ladder, SVF_MIN_DAMPING, Svf, coefficient, stage_gain};
use crate::math::{Rng, cos_turns, exp, exp2, flush, key_hz, log2, sin_turns, tanh};
use crate::osc::{NO_SYNC, Osc};

/// Samples per control step at the voice's rate.
pub const CTRL: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterKind {
    Ladder,
    Svf,
}

/// Everything that shapes a voice. A spike's patch: plain fields, no parameter contract.
#[derive(Clone, Copy, Debug)]
pub struct Patch {
    // Oscillators: waveform blend (0 saw – 1 pulse), pulse width, level.
    pub shape_a: f32,
    pub pw_a: f32,
    pub level_a: f32,
    pub shape_b: f32,
    pub pw_b: f32,
    pub level_b: f32,
    /// Osc B against A, in semitones (fractional for a detune).
    pub detune_b: f32,
    pub level_sub: f32,
    /// Osc A hard-synced to B.
    pub sync: bool,
    /// Semitones added to A's pitch by the filter envelope (a sync sweep).
    pub sync_sweep: f32,
    /// Pulse width modulation depth by the LFO.
    pub pwm: f32,

    // Filter.
    pub filter: FilterKind,
    pub cutoff_hz: f32,
    /// 0 – 1.
    pub resonance: f32,
    /// Filter envelope amount in octaves.
    pub env_amount: f32,
    /// 0 – 1: share of the key's pitch the cutoff follows (from key 60).
    pub key_track: f32,
    /// Octaves of cutoff per unit of velocity.
    pub velocity_to_cutoff: f32,
    /// Input gain into the filter's saturation (1 = clean-ish, 6+ = heavy).
    pub drive: f32,
    /// Output stage gain (its `tanh`); with a small bias for even harmonics.
    pub out_drive: f32,
    /// A fast extra filter transient at note start, in octaves (the "snap").
    pub snap: f32,

    pub amp: Adsr,
    pub filt: Adsr,

    // Play.
    pub mono: bool,
    pub legato: bool,
    /// Glide time constant in seconds (mono).
    pub glide: f32,
    /// Voices per note.
    pub unison: usize,
    /// Spread of the unison voices, in cents (outermost voice).
    pub unison_detune: f32,
    /// Stereo width of the unison voices, 0 – 1.
    pub spread: f32,
    /// Vibrato depth in cents by the LFO.
    pub vibrato: f32,
    pub lfo_rate: f32,
    /// LFO to cutoff, in octaves.
    pub lfo_to_cutoff: f32,
    /// The imperfections, 0 (exact) – 1 (worn).
    pub age: f32,
    pub level: f32,
}

impl Patch {
    /// A plain, neutral patch the sketches start from.
    pub const INIT: Patch = Patch {
        shape_a: 0.0,
        pw_a: 0.5,
        level_a: 0.7,
        shape_b: 0.0,
        pw_b: 0.5,
        level_b: 0.0,
        detune_b: 0.0,
        level_sub: 0.0,
        sync: false,
        sync_sweep: 0.0,
        pwm: 0.0,
        filter: FilterKind::Ladder,
        cutoff_hz: 2_000.0,
        resonance: 0.2,
        env_amount: 0.0,
        key_track: 0.5,
        velocity_to_cutoff: 0.0,
        drive: 1.5,
        out_drive: 1.0,
        snap: 0.0,
        amp: Adsr {
            attack: 0.005,
            decay: 0.3,
            sustain: 0.8,
            release: 0.3,
        },
        filt: Adsr {
            attack: 0.005,
            decay: 0.3,
            sustain: 0.0,
            release: 0.3,
        },
        mono: false,
        legato: false,
        glide: 0.0,
        unison: 1,
        unison_detune: 0.0,
        spread: 0.0,
        vibrato: 0.0,
        lfo_rate: 5.0,
        lfo_to_cutoff: 0.0,
        age: 0.3,
        level: 0.5,
    };
}

/// Control-rate inputs shared by every voice of a synth for one block.
#[derive(Clone, Copy, Debug)]
pub struct Shared {
    /// The LFO's value, −1 – 1.
    pub lfo: f32,
}

/// Bias of the output stage: a slight asymmetry, so a pushed stage adds even harmonics.
const OUT_BIAS: f32 = 0.15;
/// Snap time constant, seconds.
const SNAP_TAU: f32 = 0.004;

#[derive(Clone, Copy, Debug)]
pub struct Voice {
    pub active: bool,
    pub released: bool,
    /// Note-on order, for stealing.
    pub order: u64,
    pub key: u8,
    velocity: f32,
    osc_a: Osc,
    osc_b: Osc,
    sub: Osc,
    pub amp_env: Env,
    filt_env: Env,
    snap: f32,
    ladder: Ladder,
    svf: Svf,
    pitch: f32,
    target_pitch: f32,
    /// Unison offset in cents and pan gains, set at note-on.
    offset_cents: f32,
    pan: [f32; 2],
    // Tolerances of the slot, fixed for the voice's life (drawn once from its seed).
    tol_tune_a: f32,
    tol_tune_b: f32,
    tol_cutoff: f32,
    tol_env: f32,
    // Drift: a slow random walk per oscillator.
    rng: Rng,
    drift: [f32; 2],
    drift_target: [f32; 2],
    drift_clock: u32,
    // Control state: increments of A, B and sub, the filter coefficient and its ramp.
    inc: [f32; 3],
    g: f32,
    g_end: f32,
    dg: f32,
    ctrl_left: usize,
}

impl Voice {
    /// A voice slot; `seed` fixes its tolerances, drift and starting phases.
    pub fn new(seed: u64) -> Self {
        let mut rng = Rng::new(seed);
        Voice {
            active: false,
            released: false,
            order: 0,
            key: 0,
            velocity: 0.0,
            osc_a: Osc::new(rng.unit()),
            osc_b: Osc::new(rng.unit()),
            sub: Osc::new(rng.unit()),
            amp_env: Env::default(),
            filt_env: Env::default(),
            snap: 0.0,
            ladder: Ladder::default(),
            svf: Svf::default(),
            pitch: 60.0,
            target_pitch: 60.0,
            offset_cents: 0.0,
            pan: [core::f32::consts::FRAC_1_SQRT_2; 2],
            tol_tune_a: rng.bipolar(),
            tol_tune_b: rng.bipolar(),
            tol_cutoff: rng.bipolar(),
            tol_env: rng.bipolar(),
            rng,
            drift: [0.0; 2],
            drift_target: [0.0; 2],
            drift_clock: 0,
            inc: [0.0; 3],
            g: 0.1,
            g_end: 0.1,
            dg: 0.0,
            ctrl_left: 0,
        }
    }

    /// Starts a note. `glide_from` is the pitch to glide from (mono), `retrigger` whether
    /// the envelopes restart (false for a legato note).
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        &mut self,
        patch: &Patch,
        key: u8,
        velocity: f32,
        order: u64,
        offset_cents: f32,
        pan: f32,
        glide_from: Option<f32>,
        retrigger: bool,
        rate: f32,
    ) {
        self.key = key;
        self.velocity = velocity.clamp(0.0, 1.0);
        self.order = order;
        self.offset_cents = offset_cents;
        // Constant-power pan: an angle of 0 – 90° (0 – 1/4 turn).
        let turns = (pan.clamp(-1.0, 1.0) + 1.0) / 8.0;
        self.pan = [cos_turns(turns), sin_turns(turns)];
        self.target_pitch = key as f32;
        self.pitch = glide_from.unwrap_or(key as f32);
        if !self.active {
            self.ladder.reset();
            self.svf.reset();
            self.ctrl_left = 0;
        }
        self.active = true;
        self.released = false;
        if retrigger {
            let scale = 1.0 + 0.12 * patch.age * self.tol_env;
            self.amp_env.gate_on(&patch.amp, rate, scale);
            self.filt_env
                .gate_on(&patch.filt, rate / CTRL as f32, scale);
            self.snap = 1.0;
        }
    }

    /// Moves a held note to another key (legato in mono), without retriggering.
    pub fn glide_to(&mut self, key: u8) {
        self.key = key;
        self.target_pitch = key as f32;
    }

    pub fn release(&mut self) {
        self.released = true;
        self.amp_env.gate_off();
        self.filt_env.gate_off();
    }

    pub fn pitch(&self) -> f32 {
        self.pitch
    }

    /// Renders `left.len()` samples (any count: the control steps carry over calls) and adds them to `left` and
    /// `right`. `rate` is the voice's sample rate (the oversampled rate).
    pub fn render(
        &mut self,
        patch: &Patch,
        shared: Shared,
        rate: f32,
        left: &mut [f32],
        right: &mut [f32],
    ) {
        if !self.active {
            return;
        }
        let ctrl_rate = rate / CTRL as f32;
        let glide_k = if patch.glide > 0.0 {
            1.0 - exp(-1.0 / (patch.glide * ctrl_rate))
        } else {
            1.0
        };
        let snap_k = 1.0 - exp(-1.0 / (SNAP_TAU * ctrl_rate));
        // Drift: a new random target every 64 control steps (about 11 ms at 96 kHz),
        // followed by a one-pole of 0.4 s: a wander of a few tenths of hertz.
        let drift_k = 1.0 - exp(-1.0 / (0.4 * ctrl_rate));
        let age = patch.age.clamp(0.0, 1.0);
        let drift_cents = 6.0 * age;
        let tol_cents = 5.0 * age;
        let (saw_a, pulse_a) = (1.0 - patch.shape_a, patch.shape_a);
        let (saw_b, pulse_b) = (1.0 - patch.shape_b, patch.shape_b);
        let pw_a = patch.pw_a + 0.4 * patch.pwm * shared.lfo;
        let pw_b = patch.pw_b + 0.4 * patch.pwm * shared.lfo;
        let k_ladder = patch.resonance.clamp(0.0, 1.0) * LADDER_MAX_K;
        let k_svf =
            SVF_MIN_DAMPING + (2.0 - SVF_MIN_DAMPING) * (1.0 - patch.resonance.clamp(0.0, 1.0));
        let out_gain = 1.0 / patch.drive.max(1.0).sqrt();
        let bias_out = tanh(OUT_BIAS);
        let vel_gain = 0.55 + 0.45 * self.velocity;

        let len = left.len().min(right.len());
        let mut i = 0;
        while i < len {
            if self.ctrl_left == 0 {
                // --- control rate ---
                self.g = self.g_end;
                self.ladder.flush();
                self.svf.flush();
                self.pitch += (self.target_pitch - self.pitch) * glide_k;
                self.drift_clock += 1;
                if self.drift_clock >= 64 {
                    self.drift_clock = 0;
                    self.drift_target = [self.rng.bipolar(), self.rng.bipolar()];
                }
                for d in 0..2 {
                    self.drift[d] += (self.drift_target[d] - self.drift[d]) * drift_k;
                }
                let fenv = self.filt_env.tick();
                self.snap -= self.snap * snap_k;
                let vib = patch.vibrato * shared.lfo;
                let base = self.pitch + (self.offset_cents + vib) / 100.0;
                let key_a = base
                    + (tol_cents * self.tol_tune_a + drift_cents * self.drift[0]) / 100.0
                    + if patch.sync {
                        patch.sync_sweep * fenv
                    } else {
                        0.0
                    };
                let key_b = base
                    + patch.detune_b
                    + (tol_cents * self.tol_tune_b + drift_cents * self.drift[1]) / 100.0;
                self.inc = [
                    key_hz(key_a) / rate,
                    key_hz(key_b) / rate,
                    key_hz(base + (tol_cents * self.tol_tune_a) / 100.0 - 12.0) / rate,
                ];
                let octaves = log2(patch.cutoff_hz.max(10.0))
                    + 0.15 * age * self.tol_cutoff
                    + patch.key_track * (self.pitch - 60.0) / 12.0
                    + patch.env_amount * fenv
                    + patch.snap * self.snap
                    + patch.velocity_to_cutoff * self.velocity
                    + patch.lfo_to_cutoff * shared.lfo;
                let g = coefficient(exp2(octaves), rate);
                // The ladder interpolates its stage gain, the state-variable filter `g`.
                self.g_end = match patch.filter {
                    FilterKind::Ladder => stage_gain(g),
                    FilterKind::Svf => g,
                };
                self.dg = (self.g_end - self.g) / CTRL as f32;
                self.ctrl_left = CTRL;
            }
            let m = (len - i).min(self.ctrl_left);
            let [inc_a, inc_b, inc_sub] = self.inc;
            // --- audio rate ---
            for (l, r) in left[i..i + m].iter_mut().zip(right[i..i + m].iter_mut()) {
                let (b, wrap) = self.osc_b.step(inc_b, saw_b, pulse_b, pw_b, NO_SYNC);
                let sync = if patch.sync { wrap } else { NO_SYNC };
                let (a, _) = self.osc_a.step(inc_a, saw_a, pulse_a, pw_a, sync);
                let (s, _) = self.sub.step(inc_sub, 0.0, 1.0, 0.5, NO_SYNC);
                let x = a * patch.level_a + b * patch.level_b + s * patch.level_sub;
                self.g += self.dg;
                let y = match patch.filter {
                    FilterKind::Ladder => self.ladder.process(x * patch.drive, self.g, k_ladder),
                    FilterKind::Svf => self.svf.process(tanh(x * patch.drive), self.g, k_svf),
                };
                let o = (tanh(y * patch.out_drive + OUT_BIAS) - bias_out) * out_gain;
                let amp = self.amp_env.tick() * vel_gain;
                // Flushed per sample: the filter states are flushed per control step only.
                let v = flush(o * amp);
                *l += v * self.pan[0];
                *r += v * self.pan[1];
            }
            i += m;
            self.ctrl_left -= m;
        }
        if self.amp_env.is_idle() {
            self.active = false;
        }
    }
}
