//! The Compressor as it was before #157 (`crates/engine/src/devices/compressor.rs` at
//! 2301aeb9, the MVP device of DV-5), frozen: the reference render the extended device at
//! its neutral defaults is compared with, bit for bit (`tests/it/compressor.rs`). Its
//! descriptor is cut down to the bounds, defaults and ramps its parameters had (the values
//! `ParamDescriptor::sanitize` and `smoothing_frames` read), and its `Device` implementation
//! to the render, the parameter changes and the reset. Do not edit.

#![allow(dead_code)]

use engine::param::Smoother;

pub const THRESHOLD: u32 = 0;
pub const RATIO: u32 = 1;
pub const ATTACK: u32 = 2;
pub const RELEASE: u32 = 3;
pub const MAKEUP: u32 = 4;

pub const KNEE_DB: f32 = 6.0;
pub const MAKEUP_MAX_DB: f64 = 24.0;

const INPUT_LIMIT: f32 = 1e6;
const DENORMAL_FLOOR: f32 = 1e-20;
const LEVEL_FLOOR: f32 = 1e-6;
const STATE_FLOOR_DB: f32 = 1e-6;
const DB_PER_NEPER: f32 = 8.685_89;
const NEPER_PER_DB: f32 = 0.115_129_26;

/// (min, max, default, smoothing seconds) of each parameter, by id, as the MVP declared them.
const PARAMS: [(f64, f64, f64, f64); 5] = [
    (-60.0, 0.0, -18.0, 0.02),
    (1.0, 20.0, 4.0, 0.02),
    (0.0001, 0.2, 0.01, 0.0),
    (0.01, 2.0, 0.15, 0.0),
    (0.0, MAKEUP_MAX_DB, 0.0, 0.02),
];

fn sanitize_param(id: u32, value: f32) -> Option<f32> {
    let (min, max, _, _) = *PARAMS.get(id as usize)?;
    let value = f64::from(value);
    if !value.is_finite() {
        return None;
    }
    Some(value.clamp(min, max) as f32)
}

fn smoothing_frames(id: u32, sample_rate: f32) -> u32 {
    let frames = (PARAMS[id as usize].3 * f64::from(sample_rate)).round();
    if frames.is_finite() && frames > 0.0 {
        frames.min(f64::from(u32::MAX)) as u32
    } else {
        0
    }
}

fn sanitize(x: f32) -> f32 {
    if !x.is_finite() || x.abs() < DENORMAL_FLOOR {
        0.0
    } else {
        x.clamp(-INPUT_LIMIT, INPUT_LIMIT)
    }
}

fn flush_db(x: f32) -> f32 {
    if x < STATE_FLOOR_DB { 0.0 } else { x }
}

fn coefficient(seconds: f32, sample_rate: f32) -> f32 {
    (-1.0 / (f64::from(seconds.max(1e-5)) * f64::from(sample_rate))).exp() as f32
}

pub fn gain_reduction_db(x_db: f32, threshold_db: f32, slope: f32) -> f32 {
    let over = x_db - threshold_db;
    if 2.0 * over <= -KNEE_DB {
        0.0
    } else if 2.0 * over >= KNEE_DB {
        slope * over
    } else {
        let t = over + 0.5 * KNEE_DB;
        slope * t * t / (2.0 * KNEE_DB)
    }
}

#[derive(Debug)]
pub struct Compressor {
    sample_rate: f32,
    threshold: Smoother,
    ratio: Smoother,
    makeup: Smoother,
    ramp_frames: [u32; 3],
    attack: f32,
    release: f32,
    hold: f32,
    reduction: f32,
}

impl Compressor {
    pub fn new(sample_rate: f32) -> Self {
        let sample_rate = if sample_rate.is_finite() && sample_rate >= 8_000.0 {
            sample_rate
        } else {
            48_000.0
        };
        let default = |id: u32| PARAMS[id as usize].2;
        let ramp = |id: u32| smoothing_frames(id, sample_rate);
        Self {
            sample_rate,
            threshold: Smoother::new(default(THRESHOLD) as f32),
            ratio: Smoother::new(default(RATIO) as f32),
            makeup: Smoother::new(default(MAKEUP) as f32),
            ramp_frames: [ramp(THRESHOLD), ramp(RATIO), ramp(MAKEUP)],
            attack: coefficient(default(ATTACK) as f32, sample_rate),
            release: coefficient(default(RELEASE) as f32, sample_rate),
            hold: 0.0,
            reduction: 0.0,
        }
    }

    pub fn set_param(&mut self, id: u32, value: f32) {
        let Some(value) = sanitize_param(id, value) else {
            return;
        };
        match id {
            THRESHOLD => self.threshold.set(value, self.ramp_frames[0]),
            RATIO => self.ratio.set(value, self.ramp_frames[1]),
            ATTACK => self.attack = coefficient(value, self.sample_rate),
            RELEASE => self.release = coefficient(value, self.sample_rate),
            MAKEUP => self.makeup.set(value, self.ramp_frames[2]),
            _ => {}
        }
    }

    pub fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            let threshold = self.threshold.next_value().clamp(-60.0, 0.0);
            let ratio = self.ratio.next_value().clamp(1.0, 20.0);
            let makeup = self.makeup.next_value().clamp(0.0, MAKEUP_MAX_DB as f32);
            let (dry_l, dry_r) = (sanitize(*l), sanitize(*r));
            let level = dry_l.abs().max(dry_r.abs()).max(LEVEL_FLOOR);
            let x_db = DB_PER_NEPER * level.ln();
            let wanted = gain_reduction_db(x_db, threshold, 1.0 - 1.0 / ratio);
            self.hold =
                flush_db(wanted.max(self.release * self.hold + (1.0 - self.release) * wanted));
            self.reduction = if self.hold <= self.reduction {
                self.hold
            } else {
                flush_db(self.attack * self.reduction + (1.0 - self.attack) * self.hold)
            };
            let gain = (NEPER_PER_DB * (makeup - self.reduction)).exp();
            *l = flush(dry_l * gain);
            *r = flush(dry_r * gain);
        }
    }

    pub fn gain_reduction_db(&self) -> f32 {
        if self.reduction.is_finite() {
            self.reduction.max(0.0)
        } else {
            0.0
        }
    }

    pub fn reset(&mut self) {
        self.hold = 0.0;
        self.reduction = 0.0;
        self.threshold.snap();
        self.ratio.snap();
        self.makeup.snap();
    }
}

fn flush(x: f32) -> f32 {
    if x.abs() < DENORMAL_FLOOR { 0.0 } else { x }
}
