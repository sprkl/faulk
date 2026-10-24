//! A polyphonic (or mono) synth of [`MAX_VOICES`] voice slots running at the oversampled
//! rate, the 2× decimator, a DC blocker and the chorus. Everything is allocated in
//! [`Synth::new`]; [`Synth::note_on`], [`Synth::note_off`] and [`Synth::render`] never
//! allocate.

use crate::chorus::{Chorus, ChorusSettings};
use crate::halfband::Decimator;
/// The DC blocker's state is zeroed below this (-200 dB).
const DC_FLOOR: f32 = 1e-10;
use crate::voice::{Patch, Shared, Voice};

pub const MAX_VOICES: usize = 16;
/// Largest block [`Synth::render`] takes, at the output rate.
pub const MAX_BLOCK: usize = 128;
const MAX_HELD: usize = 16;

/// The voices' rate against the output rate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Oversample {
    X1,
    X2,
}

impl Oversample {
    pub fn factor(self) -> usize {
        match self {
            Oversample::X1 => 1,
            Oversample::X2 => 2,
        }
    }
}

pub struct Synth {
    pub patch: Patch,
    fs: f32,
    os: Oversample,
    voices: [Voice; MAX_VOICES],
    order: u64,
    held: [u8; MAX_HELD],
    held_len: usize,
    last_pitch: Option<f32>,
    buf: [[f32; MAX_BLOCK * 2]; 2],
    dec: [Decimator; 2],
    dc: [[f32; 2]; 2],
    chorus: Chorus,
    lfo_phase: f32,
}

impl Synth {
    /// `seed` fixes the tolerances and drift of the voice slots.
    pub fn new(fs: f32, os: Oversample, patch: Patch, chorus: ChorusSettings, seed: u64) -> Self {
        let voices = core::array::from_fn(|i| {
            Voice::new(seed.wrapping_mul(0x100).wrapping_add(i as u64 + 1))
        });
        Synth {
            patch,
            fs,
            os,
            voices,
            order: 0,
            held: [0; MAX_HELD],
            held_len: 0,
            last_pitch: None,
            buf: [[0.0; MAX_BLOCK * 2]; 2],
            dec: [Decimator::new(), Decimator::new()],
            dc: [[0.0; 2]; 2],
            chorus: Chorus::new(fs, chorus),
            lfo_phase: 0.0,
        }
    }

    fn voice_rate(&self) -> f32 {
        self.fs * self.os.factor() as f32
    }

    /// Unison offset (cents) and pan of voice `i` of `n`.
    fn unison_slot(&self, i: usize, n: usize) -> (f32, f32) {
        if n <= 1 {
            return (0.0, 0.0);
        }
        let t = 2.0 * i as f32 / (n - 1) as f32 - 1.0;
        (t * self.patch.unison_detune, t * self.patch.spread)
    }

    /// Picks a slot: a free one, else the oldest released, else the oldest.
    fn pick(&self) -> usize {
        let mut best = 0;
        let mut best_key = (3u8, u64::MAX);
        for (i, v) in self.voices.iter().enumerate() {
            let rank = if !v.active {
                0
            } else if v.released {
                1
            } else {
                2
            };
            let key = (rank, v.order);
            if key < best_key {
                best_key = key;
                best = i;
            }
        }
        best
    }

    pub fn note_on(&mut self, key: u8, velocity: f32) {
        let rate = self.voice_rate();
        let patch = self.patch;
        let n = patch.unison.clamp(1, MAX_VOICES);
        self.order += 1;
        if patch.mono {
            let was_held = self.held_len > 0;
            if self.held_len < MAX_HELD {
                self.held[self.held_len] = key;
                self.held_len += 1;
            }
            let legato = patch.legato && was_held;
            let glide_from = if patch.glide > 0.0 {
                self.last_pitch
            } else {
                None
            };
            for i in 0..n {
                let (offset, pan) = self.unison_slot(i, n);
                let v = &mut self.voices[i];
                if legato && v.active && !v.released {
                    v.glide_to(key);
                } else {
                    let from = if v.active {
                        Some(v.pitch())
                    } else {
                        glide_from
                    };
                    v.start(
                        &patch, key, velocity, self.order, offset, pan, from, true, rate,
                    );
                }
            }
            self.last_pitch = Some(key as f32);
            return;
        }
        for i in 0..n {
            let (offset, pan) = self.unison_slot(i, n);
            let slot = self.pick();
            self.voices[slot].start(
                &patch, key, velocity, self.order, offset, pan, None, true, rate,
            );
        }
    }

    pub fn note_off(&mut self, key: u8) {
        if self.patch.mono {
            let mut j = 0;
            for i in 0..self.held_len {
                if self.held[i] != key {
                    self.held[j] = self.held[i];
                    j += 1;
                }
            }
            self.held_len = j;
            let n = self.patch.unison.clamp(1, MAX_VOICES);
            if let Some(&back) = self.held[..self.held_len].last() {
                for v in &mut self.voices[..n] {
                    v.glide_to(back);
                }
                self.last_pitch = Some(back as f32);
            } else {
                for v in &mut self.voices[..n] {
                    v.release();
                }
            }
            return;
        }
        for v in &mut self.voices {
            if v.active && !v.released && v.key == key {
                v.release();
            }
        }
    }

    /// Whether any state of the synth (voices, decimators, DC blocker, chorus) is
    /// subnormal: a check for the tests, which only see the output otherwise.
    pub fn has_subnormal_state(&self) -> bool {
        self.voices.iter().any(Voice::has_subnormal_state)
            || self.dec.iter().any(Decimator::has_subnormal)
            || self.dc.iter().flatten().any(|v| v.is_subnormal())
            || self.chorus.has_subnormal()
    }

    pub fn active_voices(&self) -> usize {
        self.voices.iter().filter(|v| v.active).count()
    }

    /// Renders one block (at most [`MAX_BLOCK`] frames, any count), writing
    /// `left` and `right`.
    pub fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        let frames = left.len().min(right.len()).min(MAX_BLOCK);
        let os = self.os.factor();
        let padded = frames * os;
        let rate = self.voice_rate();
        let [bl, br] = &mut self.buf;
        let (bl, br) = (&mut bl[..padded], &mut br[..padded]);
        bl.fill(0.0);
        br.fill(0.0);

        let lfo = crate::math::sin_turns(self.lfo_phase);
        self.lfo_phase = (self.lfo_phase + self.patch.lfo_rate * frames as f32 / self.fs).fract();
        let shared = Shared { lfo };
        for v in &mut self.voices {
            v.render(&self.patch, shared, rate, bl, br);
        }

        let level = self.patch.level;
        for ch in 0..2 {
            let src = if ch == 0 { &*bl } else { &*br };
            let out = if ch == 0 { &mut *left } else { &mut *right };
            let [x1, y1] = &mut self.dc[ch];
            for (i, o) in out[..frames].iter_mut().enumerate() {
                let x = match self.os {
                    Oversample::X1 => src[i],
                    Oversample::X2 => self.dec[ch].process(src[2 * i], src[2 * i + 1]),
                };
                // DC blocker (the output stage's bias adds a little DC).
                let y = x - *x1 + 0.9995 * *y1;
                *x1 = x;
                // Its tail decays at 0.9995 per sample: cut it at -200 dB, not at the
                // denormal guard, so that silence comes back within a second.
                *y1 = if y.abs() < DC_FLOOR { 0.0 } else { y };
                *o = y * level;
            }
        }
        self.chorus
            .process(&mut left[..frames], &mut right[..frames]);
    }
}
