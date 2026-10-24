//! The CPU workload, the same natively and in WASM: `voices` notes held on one synth for
//! `seconds` of audio at 48 kHz, in blocks of 128 frames. Each voice runs two saws and the
//! sub through the filter, with a filter envelope still moving, drift on, and the chorus
//! on, so the shared costs (decimator, DC blocker, chorus) are counted once per synth.

use crate::chorus::ChorusSettings;
use crate::env::Adsr;
use crate::synth::{MAX_BLOCK, Oversample, Synth};
use crate::voice::{FilterKind, Patch};

pub const FS: f32 = 48_000.0;

pub fn patch(filter: FilterKind) -> Patch {
    Patch {
        level_a: 0.5,
        level_b: 0.5,
        detune_b: 0.07,
        level_sub: 0.3,
        filter,
        cutoff_hz: 800.0,
        resonance: 0.6,
        env_amount: 3.0,
        drive: 3.0,
        out_drive: 1.5,
        amp: Adsr {
            attack: 0.01,
            decay: 1.0,
            sustain: 0.8,
            release: 0.5,
        },
        filt: Adsr {
            attack: 0.5,
            decay: 4.0,
            sustain: 0.3,
            release: 0.5,
        },
        age: 0.5,
        level: 0.1,
        ..Patch::INIT
    }
}

/// Variant index: bit 0 the filter (0 ladder, 1 state-variable), bit 1 the oversampling
/// (0 for 2×, 1 for 1×).
pub fn variant(index: u32) -> (FilterKind, Oversample) {
    let filter = if index & 1 == 0 {
        FilterKind::Ladder
    } else {
        FilterKind::Svf
    };
    let os = if index & 2 == 0 {
        Oversample::X2
    } else {
        Oversample::X1
    };
    (filter, os)
}

/// A synth with `voices` notes held, ready to render.
pub fn prepare(index: u32, voices: usize) -> Synth {
    let (filter, os) = variant(index);
    let chorus = ChorusSettings {
        mix: 0.5,
        ..ChorusSettings::OFF
    };
    let mut synth = Synth::new(FS, os, patch(filter), chorus, 349);
    for i in 0..voices {
        synth.note_on(36 + (i as u8 * 5) % 48, 0.8);
    }
    synth
}

/// Renders `seconds` of audio and returns a checksum (so that nothing is optimised away).
pub fn run(synth: &mut Synth, seconds: f32) -> f32 {
    let blocks = (seconds * FS / MAX_BLOCK as f32) as usize;
    let mut left = [0.0f32; MAX_BLOCK];
    let mut right = [0.0f32; MAX_BLOCK];
    let mut sum = 0.0f32;
    for _ in 0..blocks {
        synth.render(&mut left, &mut right);
        sum += left[0] + right[MAX_BLOCK - 1];
    }
    sum
}
