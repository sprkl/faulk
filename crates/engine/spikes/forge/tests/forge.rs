//! The spike's checks: bounded self-oscillation, no NaN or denormals at the parameters'
//! extremes, no allocation on the audio path, deterministic renders, measured aliasing.
//! Run with `cargo test` (the dev profile is optimised; the allocation detector needs debug
//! assertions).

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use forge_spike::analysis::{SawKind, alias_report, bad_samples, peak, rms, saw};
use forge_spike::chorus::ChorusSettings;
use forge_spike::filter::{LADDER_MAX_K, Ladder, coefficient, stage_gain};
use forge_spike::song::{Performance, SKETCHES};
use forge_spike::synth::{MAX_BLOCK, Oversample, Synth};
use forge_spike::voice::{FilterKind, Patch};

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

#[test]
fn ladder_self_oscillates_and_stays_bounded() {
    let fs = 96_000.0;
    for cutoff in [30.0, 200.0, 1_000.0, 5_000.0, 15_000.0, 24_000.0] {
        let big_g = stage_gain(coefficient(cutoff, fs));
        // An impulse, then silence: the oscillation must start and stay bounded.
        let mut l = Ladder::default();
        let mut out = Vec::with_capacity(fs as usize * 3);
        for i in 0..(fs as usize * 3) {
            let x = if i == 0 { 1.0 } else { 0.0 };
            out.push(l.process(x, big_g, LADDER_MAX_K));
        }
        let (bad, den) = bad_samples(&out);
        assert_eq!((bad, den), (0, 0), "cutoff {cutoff}");
        assert!(peak(&out) <= 1.0, "cutoff {cutoff}: peak {}", peak(&out));
        let tail = rms(&out[out.len() - fs as usize..]);
        assert!(
            tail > 0.05,
            "cutoff {cutoff}: no self-oscillation (rms {tail})"
        );

        // Driven by a loud square far above the oscillation: still bounded.
        let mut l = Ladder::default();
        for i in 0..(fs as usize) {
            let x = if (i / 37) % 2 == 0 { 20.0 } else { -20.0 };
            let y = l.process(x, big_g, LADDER_MAX_K);
            assert!(y.is_finite() && y.abs() <= 1.0, "cutoff {cutoff}: {y}");
        }
    }
}

/// Renders `seconds` of a chord of `keys` (unison included) on a fresh synth, then the
/// release, block by block.
fn render_chord(patch: Patch, os: Oversample, keys: &[u8], seconds: f32) -> (Vec<f32>, Vec<f32>) {
    let fs = 48_000.0;
    let chorus = ChorusSettings {
        mix: 0.6,
        ..ChorusSettings::OFF
    };
    let mut synth = Synth::new(fs, os, patch, chorus, 7);
    let frames = (seconds * fs) as usize;
    // The note, then 4.5 s of release and silence (the longest release here is 1.6 s,
    // over 99 % of the way in that time, to -80 dB in 1.85 times it; then the DC blocker).
    let total = frames + 216_000;
    let mut l = vec![0.0f32; total];
    let mut r = vec![0.0f32; total];
    for &k in keys {
        synth.note_on(k, 1.0);
    }
    // Odd block sizes on purpose: the control steps carry over blocks.
    let mut at = 0;
    let mut released = false;
    while at < l.len() {
        if !released && at >= frames {
            for &k in keys {
                synth.note_off(k);
            }
            released = true;
        }
        let n = (37 + at % 91).min(MAX_BLOCK).min(l.len() - at);
        synth.render(&mut l[at..at + n], &mut r[at..at + n]);
        at += n;
    }
    (l, r)
}

#[test]
fn extremes_stay_finite_bounded_and_free_of_denormals() {
    let keys = [24u8, 36, 48, 60, 72, 84, 96, 108];
    let mut cases = Vec::new();
    for sketch in &SKETCHES {
        for part in [sketch.bass, sketch.pad, sketch.lead] {
            cases.push(part.patch);
        }
    }
    let mut worst = 0.0f32;
    for base in cases {
        for filter in [FilterKind::Ladder, FilterKind::Svf] {
            for (cutoff, resonance, drive, age) in [
                (10.0, 0.0, 0.5, 0.0),
                (20_000.0, 1.0, 20.0, 1.0),
                (10.0, 1.0, 20.0, 1.0),
                (20_000.0, 0.0, 0.5, 0.0),
                (base.cutoff_hz, 1.0, base.drive, 1.0),
            ] {
                for os in [Oversample::X1, Oversample::X2] {
                    let patch = Patch {
                        filter,
                        cutoff_hz: cutoff,
                        resonance,
                        drive,
                        age,
                        out_drive: 5.0,
                        unison: 3,
                        ..base
                    };
                    let (l, r) = render_chord(patch, os, &keys, 0.5);
                    for ch in [&l, &r] {
                        assert_eq!(bad_samples(ch), (0, 0), "{patch:?}");
                        let p = peak(ch) / patch.level;
                        worst = worst.max(p);
                        assert!(p < 40.0, "{patch:?}: peak / level {p}");
                        // After the release, true silence: no tail decaying into denormals.
                        let end = &ch[ch.len() - 2_400..];
                        assert!(end.iter().all(|&v| v == 0.0), "{patch:?}: tail not silent");
                    }
                }
            }
        }
    }
    println!("worst peak / level: {worst}");
}

#[test]
fn sketches_render_without_allocating_and_deterministically() {
    for sketch in &SKETCHES {
        let mut first = Performance::new(sketch, 48_000.0, Oversample::X2, 349);
        let mut second = Performance::new(sketch, 48_000.0, Oversample::X2, 349);
        let frames = first.frames;
        let (mut l1, mut r1) = (vec![0.0f32; frames], vec![0.0f32; frames]);
        let (mut l2, mut r2) = (vec![0.0f32; frames], vec![0.0f32; frames]);
        assert_no_alloc(|| first.render_into(&mut l1, &mut r1));
        assert_no_alloc(|| second.render_into(&mut l2, &mut r2));
        assert_eq!(bad_samples(&l1), (0, 0));
        assert_eq!(bad_samples(&r1), (0, 0));
        assert!(rms(&l1) > 0.01, "sketch {} is silent", sketch.name);
        assert!(
            l1.iter().zip(&l2).all(|(a, b)| a.to_bits() == b.to_bits())
                && r1.iter().zip(&r2).all(|(a, b)| a.to_bits() == b.to_bits()),
            "sketch {} is not deterministic",
            sketch.name
        );
    }
}

#[test]
fn polyblep_saw_aliasing_at_4_1_khz() {
    let fs = 48_000.0;
    let at = |kind, os| {
        alias_report(
            &saw(kind, os, 4_100.0, fs, 1 << 16),
            fs as f64,
            4_100.0,
            19_000.0,
        )
    };
    let naive = at(SawKind::Naive, Oversample::X1);
    let blep = at(SawKind::PolyBlep, Oversample::X1);
    let blep2 = at(SawKind::PolyBlep, Oversample::X2);
    // Thresholds a little under what was measured (see the spike report).
    assert!(naive.worst_audible_dbc > -20.0);
    assert!(blep.worst_audible_dbc < -32.0, "{blep:?}");
    assert!(blep2.worst_audible_dbc < -50.0, "{blep2:?}");
    assert!(blep2.alias_to_harmonics_db < -26.0, "{blep2:?}");
}
