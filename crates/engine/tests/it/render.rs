//! Offline render tests: they assert on the signal, without an audio device.

use assert_no_alloc::assert_no_alloc;
use engine::{MAX_VOICES, Param, Synth, Waveform};

const SR: f32 = 48_000.0;

fn render(synth: &mut Synth, frames: usize) -> Vec<f32> {
    let mut left = vec![0.0; frames];
    let mut right = vec![0.0; frames];
    synth.render(&mut left, &mut right);
    assert_eq!(left, right, "the synth is mono: both channels must match");
    left
}

fn assert_clean(signal: &[f32]) {
    for (i, s) in signal.iter().enumerate() {
        assert!(s.is_finite(), "sample {i} is not finite: {s}");
        assert!(s.abs() <= 1.0, "sample {i} exceeds full scale: {s}");
        assert!(*s == 0.0 || s.abs() >= 1e-20, "sample {i} is denormal: {s}");
    }
}

fn peak(signal: &[f32]) -> f32 {
    signal.iter().fold(0.0f32, |m, s| m.max(s.abs()))
}

/// Frequency estimated from rising zero crossings.
fn frequency(signal: &[f32]) -> f32 {
    let crossings: Vec<usize> = signal
        .windows(2)
        .enumerate()
        .filter(|(_, w)| w[0] < 0.0 && w[1] >= 0.0)
        .map(|(i, _)| i)
        .collect();
    let periods = (crossings.len() - 1) as f32;
    let samples = (crossings[crossings.len() - 1] - crossings[0]) as f32;
    SR * periods / samples
}

#[test]
fn renders_silence_without_notes() {
    let mut synth = Synth::new(SR);
    let out = render(&mut synth, 4_800);
    assert!(out.iter().all(|s| *s == 0.0));
}

#[test]
fn sine_a4_plays_at_440_hz() {
    let mut synth = Synth::new(SR);
    synth.set_param(Param::Waveform, Waveform::Sine.index() as f32);
    synth.note_on(69, 100);
    let _attack = render(&mut synth, 4_800);
    let out = render(&mut synth, 48_000);
    assert_clean(&out);
    let f = frequency(&out);
    assert!((f - 440.0).abs() < 0.5, "measured {f} Hz");
    assert!(peak(&out) > 0.1, "the note must be audible");
}

#[test]
fn every_waveform_plays_the_right_pitch() {
    for index in 0..4 {
        let mut synth = Synth::new(SR);
        synth.set_param(Param::Waveform, index as f32);
        synth.note_on(57, 127); // A3, 220 Hz
        let _attack = render(&mut synth, 4_800);
        let out = render(&mut synth, 48_000);
        assert_clean(&out);
        let f = frequency(&out);
        assert!((f - 220.0).abs() < 1.0, "waveform {index}: measured {f} Hz");
    }
}

#[test]
fn release_ends_in_exact_silence_without_click() {
    let mut synth = Synth::new(SR);
    synth.set_param(Param::Waveform, Waveform::Sine.index() as f32);
    synth.set_param(Param::Release, 0.05);
    synth.note_on(60, 127);
    let held = render(&mut synth, 9_600);
    synth.note_off(60);
    let released = render(&mut synth, 9_600);
    assert_clean(&released);

    // The tail is exact silence and the voice is freed.
    assert!(released[4_800..].iter().all(|s| *s == 0.0));
    assert_eq!(synth.active_voices(), 0);

    // No jump at the note off boundary: the step between two samples stays
    // within what the oscillator itself produces.
    let max_step = |s: &[f32]| {
        s.windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0, f32::max)
    };
    let boundary = (released[0] - held[held.len() - 1]).abs();
    assert!(
        boundary <= max_step(&held) * 1.01,
        "jump of {boundary} at note off"
    );
    assert!(max_step(&released) <= max_step(&held) * 1.01);
}

#[test]
fn note_start_has_no_click() {
    let mut synth = Synth::new(SR);
    synth.set_param(Param::Waveform, Waveform::Square.index() as f32);
    synth.set_param(Param::Attack, 0.0); // clamped to the minimum attack
    synth.note_on(60, 127);
    let out = render(&mut synth, 480);
    // The first sample is near zero: the attack ramps from silence.
    assert!(out[0].abs() < 0.01, "first sample {}", out[0]);
}

#[test]
fn chords_and_voice_stealing_stay_clean() {
    let mut synth = Synth::new(SR);
    synth.set_param(Param::Waveform, Waveform::Saw.index() as f32);
    for note in 40..(40 + MAX_VOICES as u8 + 8) {
        synth.note_on(note, 127);
        let out = render(&mut synth, 256);
        assert_clean(&out);
    }
    assert!(synth.active_voices() <= MAX_VOICES);
    let out = render(&mut synth, 4_800);
    assert_clean(&out);
    assert!(peak(&out) > 0.1);
}

#[test]
fn three_note_chord_is_louder_than_one_note() {
    let mut one = Synth::new(SR);
    one.set_param(Param::Waveform, Waveform::Sine.index() as f32);
    one.note_on(60, 100);
    let mut three = one.clone();
    three.note_on(64, 100);
    three.note_on(67, 100);
    let _ = render(&mut one, 4_800);
    let _ = render(&mut three, 4_800);
    let rms = |s: &[f32]| (s.iter().map(|x| x * x).sum::<f32>() / s.len() as f32).sqrt();
    assert!(rms(&render(&mut three, 48_000)) > rms(&render(&mut one, 48_000)) * 1.4);
}

#[test]
fn parameters_change_while_playing() {
    let mut synth = Synth::new(SR);
    synth.note_on(60, 100);
    let _ = render(&mut synth, 4_800);
    let before = peak(&render(&mut synth, 4_800));
    synth.set_param(Param::Gain, 0.1);
    let _ = render(&mut synth, 4_800); // gain smoothing settles
    let after = peak(&render(&mut synth, 4_800));
    assert!(
        after < before * 0.3,
        "gain did not apply: {before} -> {after}"
    );
}

#[test]
fn invalid_input_is_ignored_or_clamped() {
    let mut synth = Synth::new(f32::NAN);
    assert_eq!(synth.sample_rate(), 48_000.0);
    synth.set_param(Param::Gain, f32::NAN);
    synth.set_param(Param::Sustain, 7.0);
    assert_eq!(synth.param(Param::Sustain), 1.0);
    synth.set_param(Param::Waveform, -3.0);
    assert_eq!(synth.param(Param::Waveform), 0.0);
    synth.note_on(200, 200);
    synth.note_on(60, 0); // velocity 0 is a note off
    let mut left = vec![0.0; 64];
    let mut right = vec![0.0; 32]; // mismatched lengths render the common part
    synth.render(&mut left, &mut right);
    assert_clean(&left);
}

#[test]
fn audio_path_never_allocates() {
    let mut synth = Synth::new(SR);
    let mut left = vec![0.0; 128];
    let mut right = vec![0.0; 128];
    assert_no_alloc(|| {
        // 10 seconds of 128-frame quanta with notes and parameter changes.
        for block in 0..3_750u32 {
            if block % 50 == 0 {
                synth.note_on((block / 50 % 48 + 36) as u8, 100);
            }
            if block % 50 == 25 {
                synth.note_off((block / 50 % 48 + 36) as u8);
            }
            if block % 200 == 0 {
                synth.set_param(Param::Waveform, (block / 200 % 4) as f32);
                synth.set_param(Param::Release, 0.1 + (block % 7) as f32 * 0.01);
            }
            synth.render(&mut left, &mut right);
        }
        synth.all_notes_off();
        synth.render(&mut left, &mut right);
    });
}

/// Largest step between two consecutive samples.
fn max_step(signal: &[f32]) -> f32 {
    signal
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max)
}

/// A low sine changes by at most 2π·f/sr·peak per sample (≈ 0.007 at 55 Hz):
/// any jump of the envelope (a click) is far above this bound.
const CLICK_BOUND: f32 = 0.02;

fn low_sine_synth(release: f32) -> Synth {
    let mut synth = Synth::new(SR);
    synth.set_param(Param::Waveform, Waveform::Sine.index() as f32);
    synth.set_param(Param::Attack, 0.05);
    synth.set_param(Param::Decay, 0.05);
    synth.set_param(Param::Sustain, 0.5);
    synth.set_param(Param::Release, release);
    synth
}

#[test]
fn note_off_in_every_envelope_stage_has_no_click() {
    // Note off during attack (10 ms in), decay (70 ms in) and sustain (500 ms in),
    // with the shortest release.
    for held_frames in [480, 3_360, 24_000] {
        let mut synth = low_sine_synth(0.0); // clamped to the minimum release
        synth.note_on(33, 127); // A1, 55 Hz
        let held = render(&mut synth, held_frames);
        synth.note_off(33);
        let released = render(&mut synth, 4_800);
        let mut joined = held;
        joined.extend_from_slice(&released);
        assert_clean(&joined);
        let step = max_step(&joined);
        assert!(step < CLICK_BOUND, "held {held_frames} frames: step {step}");
        assert_eq!(synth.active_voices(), 0, "held {held_frames} frames");
    }
}

#[test]
fn voice_stealing_has_no_click() {
    let mut synth = low_sine_synth(10.0);
    // Sixteen distinct low notes fill every voice.
    for note in 24..24 + MAX_VOICES as u8 {
        synth.note_on(note, 127);
    }
    let steady = render(&mut synth, 9_600);
    let before = max_step(&steady[4_800..]);
    // Each extra note must steal a voice; the stolen one fades out instead of cutting.
    let mut stolen = Vec::new();
    for note in 24 + MAX_VOICES as u8..24 + MAX_VOICES as u8 + 6 {
        synth.note_on(note, 127);
        stolen.extend(render(&mut synth, 960));
    }
    assert_eq!(synth.active_voices(), MAX_VOICES);
    assert!(!synth.is_held(24), "the oldest note was stolen");
    assert!(
        synth.is_held(24 + MAX_VOICES as u8 + 5),
        "the newest note plays"
    );
    assert_clean(&stolen);
    // A cut voice would jump by its whole level (about 0.12 here), far above the
    // steady slope of the sixteen-voice sum.
    let during = max_step(&stolen);
    assert!(
        during < before * 1.5 + 0.005,
        "steady {before}, while stealing {during}"
    );
}

#[test]
fn stealing_prefers_a_released_voice_over_a_held_one() {
    let mut synth = low_sine_synth(10.0);
    for note in 24..24 + MAX_VOICES as u8 {
        synth.note_on(note, 127);
    }
    let _ = render(&mut synth, 480);
    synth.note_off(30); // still sounding: release is 10 s
    synth.note_on(60, 127);
    let _ = render(&mut synth, 480);
    assert!(synth.is_held(24), "the oldest held note keeps playing");
    assert!(synth.is_held(60));
    for note in (24..24 + MAX_VOICES as u8).filter(|n| *n != 30) {
        assert!(
            synth.is_held(note),
            "note {note} was stolen instead of the released one"
        );
    }
}

#[test]
fn changing_sustain_while_notes_are_held_has_no_click() {
    let mut synth = low_sine_synth(10.0);
    synth.note_on(33, 127);
    let mut out = render(&mut synth, 9_600); // well into sustain (0.5)
    synth.set_param(Param::Sustain, 0.05);
    out.extend(render(&mut synth, 2_400));
    synth.set_param(Param::Sustain, 1.0);
    out.extend(render(&mut synth, 2_400));
    assert_clean(&out);
    let step = max_step(&out);
    assert!(step < CLICK_BOUND, "step {step}");
}

#[test]
fn zero_gain_settles_to_exact_silence() {
    let mut synth = Synth::new(SR);
    synth.note_on(60, 127);
    let _ = render(&mut synth, 4_800);
    synth.set_param(Param::Gain, 0.0);
    let _ = render(&mut synth, 48_000);
    assert!(render(&mut synth, 4_800).iter().all(|s| *s == 0.0));
}

/// Energy of the (Hann-windowed) DFT bins that are not within two bins of a harmonic below Nyquist:
/// for a band-limited sawtooth this is aliasing (plus a little leakage).
fn alias_energy(signal: &[f32], frequency: f32) -> f64 {
    let n = signal.len();
    let bin_hz = SR as f64 / n as f64;
    let harmonics: Vec<f64> = (1..)
        .map(|k| k as f64 * frequency as f64)
        .take_while(|f| *f < SR as f64 / 2.0)
        .collect();
    let mut energy = 0.0;
    for bin in 1..n / 2 {
        let hz = bin as f64 * bin_hz;
        if harmonics.iter().any(|h| (h - hz).abs() <= 2.0 * bin_hz) {
            continue;
        }
        let (mut re, mut im) = (0.0f64, 0.0f64);
        for (t, x) in signal.iter().enumerate() {
            // Hann window, so leakage from the harmonics does not swamp the aliases.
            let w = 0.5 - 0.5 * (2.0 * std::f64::consts::PI * t as f64 / n as f64).cos();
            let a = -2.0 * std::f64::consts::PI * bin as f64 * t as f64 / n as f64;
            re += w * f64::from(*x) * a.cos();
            im += w * f64::from(*x) * a.sin();
        }
        energy += re * re + im * im;
    }
    energy
}

#[test]
fn polyblep_saw_aliases_much_less_than_a_naive_saw() {
    // A high note, where a naive sawtooth aliases badly.
    let note = 96u8;
    let frequency = 440.0 * ((f32::from(note) - 69.0) / 12.0).exp2();
    let mut synth = Synth::new(SR);
    synth.set_param(Param::Waveform, Waveform::Saw.index() as f32);
    synth.set_param(Param::Sustain, 1.0);
    synth.set_param(Param::Gain, 0.1); // stays in tanh's linear range
    synth.note_on(note, 127);
    let _ = render(&mut synth, 9_600);
    let window = 2_048;
    let engine = render(&mut synth, window);
    let naive: Vec<f32> = (0..window)
        .map(|t| 2.0 * ((t as f32 * frequency / SR) % 1.0) - 1.0)
        .collect();
    let scale = |s: &[f32]| s.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>();
    let engine_ratio = alias_energy(&engine, frequency) / scale(&engine);
    let naive_ratio = alias_energy(&naive, frequency) / scale(&naive);
    assert!(
        engine_ratio < naive_ratio * 0.5,
        "aliasing ratio: engine {engine_ratio:.4}, naive {naive_ratio:.4}"
    );
}

#[test]
fn retriggering_a_releasing_note_has_no_click() {
    let mut synth = low_sine_synth(1.0);
    synth.note_on(33, 127);
    let mut out = render(&mut synth, 9_600);
    synth.note_off(33);
    out.extend(render(&mut synth, 2_400));
    synth.note_on(33, 127);
    out.extend(render(&mut synth, 9_600));
    assert_clean(&out);
    let step = max_step(&out);
    assert!(step < CLICK_BOUND, "step {step}");
}
