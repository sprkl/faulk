//! A stereo chorus: two modulated delay taps per channel (a slow sine LFO in quadrature, so
//! the four taps move at four phases), linear interpolation, and a one-pole low-pass on the
//! wet signal for the dark tone of a bucket-brigade line. The delay lines are allocated at
//! construction; processing never allocates.

use crate::math::{cos_turns, exp, flush, sin_turns};

#[derive(Clone, Copy, Debug)]
pub struct ChorusSettings {
    /// Wet amount, 0 (off) – 1.
    pub mix: f32,
    /// LFO rate in hertz.
    pub rate: f32,
    /// Modulation depth in milliseconds.
    pub depth_ms: f32,
    /// Centre delay in milliseconds.
    pub delay_ms: f32,
}

impl ChorusSettings {
    pub const OFF: ChorusSettings = ChorusSettings {
        mix: 0.0,
        rate: 0.5,
        depth_ms: 2.0,
        delay_ms: 7.0,
    };
}

pub struct Chorus {
    settings: ChorusSettings,
    left: Box<[f32]>,
    right: Box<[f32]>,
    pos: usize,
    cos: f32,
    sin: f32,
    rot_cos: f32,
    rot_sin: f32,
    lp: [f32; 2],
    lp_coef: f32,
    fs: f32,
}

const LINE: usize = 4096;

impl Chorus {
    pub fn new(fs: f32, settings: ChorusSettings) -> Self {
        let turns = settings.rate / fs;
        Chorus {
            settings,
            left: vec![0.0; LINE].into_boxed_slice(),
            right: vec![0.0; LINE].into_boxed_slice(),
            pos: 0,
            cos: 1.0,
            sin: 0.0,
            rot_cos: cos_turns(turns),
            rot_sin: sin_turns(turns),
            lp: [0.0; 2],
            lp_coef: 1.0 - exp(-core::f32::consts::TAU * 7_000.0 / fs),
            fs,
        }
    }

    #[inline(always)]
    fn read(line: &[f32], pos: usize, delay: f32) -> f32 {
        let d = delay.clamp(1.0, (LINE - 2) as f32);
        let whole = d as usize;
        let frac = d - whole as f32;
        let a = line[(pos + LINE - whole) % LINE];
        let b = line[(pos + LINE - whole - 1) % LINE];
        a + (b - a) * frac
    }

    pub fn has_subnormal(&self) -> bool {
        self.lp.iter().any(|v| v.is_subnormal())
            || self.left.iter().any(|v| v.is_subnormal())
            || self.right.iter().any(|v| v.is_subnormal())
    }

    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        let s = self.settings;
        if s.mix <= 0.0 {
            return;
        }
        let ms = self.fs / 1000.0;
        let centre = s.delay_ms * ms;
        let depth = s.depth_ms * ms;
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            // Quadrature LFO by rotation.
            let c = self.cos * self.rot_cos - self.sin * self.rot_sin;
            let sn = self.sin * self.rot_cos + self.cos * self.rot_sin;
            self.cos = c;
            self.sin = sn;
            self.pos = (self.pos + 1) % LINE;
            self.left[self.pos] = *l;
            self.right[self.pos] = *r;
            let wl = 0.5
                * (Self::read(&self.left, self.pos, centre + depth * c)
                    + Self::read(&self.left, self.pos, centre - depth * c));
            let wr = 0.5
                * (Self::read(&self.right, self.pos, centre + depth * sn)
                    + Self::read(&self.right, self.pos, centre - depth * sn));
            self.lp[0] = flush(self.lp[0] + (wl - self.lp[0]) * self.lp_coef);
            self.lp[1] = flush(self.lp[1] + (wr - self.lp[1]) * self.lp_coef);
            *l = *l * (1.0 - 0.5 * s.mix) + self.lp[0] * s.mix;
            *r = *r * (1.0 - 0.5 * s.mix) + self.lp[1] * s.mix;
        }
        // Keep the LFO on the unit circle, and the low-pass out of denormals.
        let norm = 1.5 - 0.5 * (self.cos * self.cos + self.sin * self.sin);
        self.cos *= norm;
        self.sin *= norm;
        self.lp = [flush(self.lp[0]), flush(self.lp[1])];
    }
}
