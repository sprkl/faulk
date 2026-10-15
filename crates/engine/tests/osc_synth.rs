//! The ported synth, `faulk.osc_synth` (DV-1): offline renders through the device contract,
//! every `process` under the allocation detector. Asserts on the signal: the reference
//! render of the synth as it was before DV-1 (`reference/old_synth.rs`), the declared
//! smoothing of gain and sustain, sample-accurate events, notes by voice id, fine tuning,
//! the contract's conformance run on every device of the registry, `reset` and `set_param`.

#[path = "reference/old_synth.rs"]
mod old_synth;

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use engine::device::testing::conformance;
use engine::device::{
    AudioIo, BlockCtx, Device, DeviceRole, DeviceSetup, Event, EventBuffer, Transport, VoiceId,
    builtin, builtin_descriptors,
};
use engine::devices::{REGISTRY, factory};
use engine::engine::builtin_factory;
use engine::msg::{DeviceKind, ParamId};
use engine::{OSC_SYNTH, Param, Synth};

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

const SR: f32 = 48_000.0;

fn pid(p: Param) -> ParamId {
    ParamId(p.id())
}

fn voice(id: u32) -> VoiceId {
    VoiceId::new(id).unwrap()
}

fn on(offset: u32, v: u32, key: u8, velocity: f32) -> Event {
    Event::NoteOn {
        offset,
        voice: voice(v),
        key,
        velocity,
        tune_cents: 0.0,
    }
}

fn off(offset: u32, v: u32, key: u8) -> Event {
    Event::NoteOff {
        offset,
        voice: voice(v),
        key,
    }
}

fn param(offset: u32, p: Param, value: f32) -> Event {
    Event::ParamValue {
        offset,
        id: pid(p),
        value,
    }
}

/// Renders one block of `frames` through `process`, under the allocation detector; returns
/// the left channel and checks the right one is the same.
fn block(device: &mut dyn Device, frames: usize, events: &[Event]) -> Vec<f32> {
    let mut left = vec![0.0; frames];
    let mut right = vec![0.0; frames];
    let mut unused = EventBuffer::try_with_capacity(1).unwrap();
    let ctx = BlockCtx {
        sample_rate: SR,
        frames: frames as u32,
        transport: Transport::default(),
    };
    assert_no_alloc(|| {
        let mut io = AudioIo {
            left: &mut left,
            right: &mut right,
            events_out: &mut unused,
        };
        device.process(&ctx, events, &mut io);
    });
    assert_eq!(left, right, "the synth is mono");
    left
}

/// Renders `total` frames in blocks of `size`, each event at its absolute frame.
fn timeline(
    device: &mut dyn Device,
    total: usize,
    size: usize,
    events: &[(usize, Event)],
) -> Vec<f32> {
    let mut out = Vec::with_capacity(total);
    let mut start = 0;
    while start < total {
        let frames = size.min(total - start);
        let local: Vec<Event> = events
            .iter()
            .filter(|(at, _)| (start..start + frames).contains(at))
            .map(|(at, e)| e.at((at - start) as u32))
            .collect();
        out.extend(block(device, frames, &local));
        start += frames;
    }
    out
}

fn assert_clean(signal: &[f32]) {
    for (i, s) in signal.iter().enumerate() {
        assert!(s.is_finite(), "sample {i} is not finite: {s}");
        assert!(s.abs() <= 1.0, "sample {i} exceeds full scale: {s}");
        assert!(*s == 0.0 || s.abs() >= 1e-20, "sample {i} is denormal: {s}");
    }
}

fn max_diff(a: &[f32], b: &[f32]) -> f32 {
    a.iter()
        .zip(b)
        .fold(0.0f32, |m, (x, y)| m.max((x - y).abs()))
}

// ---------------------------------------------------------------- the reference render

/// A step of a performance, played the same way on the old synth and on the device.
#[derive(Clone, Copy)]
enum Step {
    On(u8, u8),
    Off(u8),
    AllOff,
    Set(Param, f32),
    Render(usize),
}

/// The old synth plays a performance through its own calls.
fn render_old(steps: &[Step]) -> Vec<f32> {
    let mut synth = old_synth::Synth::new(SR);
    let mut out = Vec::new();
    for step in steps {
        match *step {
            Step::On(k, v) => synth.note_on(k, v),
            Step::Off(k) => synth.note_off(k),
            Step::AllOff => synth.all_notes_off(),
            Step::Set(p, v) => synth.set_param(old_synth::Param::from_id(p.id()).unwrap(), v),
            Step::Render(n) => {
                let mut l = vec![0.0; n];
                let mut r = vec![0.0; n];
                synth.render(&mut l, &mut r);
                out.extend(l);
            }
        }
    }
    out
}

/// The ported device plays the same performance as events (the graph's mapping of a live
/// note: voice id `key + 1`, velocity `v / 127`), in blocks of `size`.
fn render_new(steps: &[Step], size: usize) -> Vec<f32> {
    let mut device = factory(OSC_SYNTH.kind_id).unwrap()(&DeviceSetup::default()).unwrap();
    let mut events = Vec::new();
    let mut at = 0;
    for step in steps {
        match *step {
            Step::On(k, v) => events.push((at, on(0, u32::from(k) + 1, k, f32::from(v) / 127.0))),
            Step::Off(k) => events.push((at, off(0, u32::from(k) + 1, k))),
            Step::AllOff => events.push((at, Event::AllNotesOff { offset: 0 })),
            Step::Set(p, v) => events.push((at, param(0, p, v))),
            Step::Render(n) => at += n,
        }
    }
    timeline(device.as_mut(), at, size, &events)
}

/// A performance with constant gain and sustain: chords, a retrigger, a note-on of velocity
/// 0, more notes than voices (stealing), waveform and envelope changes, every waveform.
fn constant_performance() -> Vec<Step> {
    use Step::*;
    let mut steps = vec![
        On(60, 100),
        Render(4_000),
        On(64, 90),
        On(67, 80),
        Render(9_000),
        Set(Param::Waveform, 2.0),
        Set(Param::Attack, 0.05),
        On(72, 127),
        Render(6_000),
        Off(60),
        On(60, 70), // retrigger of a releasing key
        Render(3_000),
        Set(Param::Waveform, 3.0),
        Set(Param::Decay, 0.05),
        On(76, 0), // velocity 0: a note-off
        Render(5_000),
        Set(Param::Waveform, 0.0),
        Set(Param::Release, 0.05),
        Off(64),
        Off(67),
        Render(3_000),
    ];
    // Twenty notes: four steal a voice.
    steps.push(Set(Param::Waveform, 1.0));
    for k in 40..60 {
        steps.push(On(k, 60 + k));
        steps.push(Render(300));
    }
    steps.push(Set(Param::Waveform, 1.7)); // rounded to 2
    steps.push(Render(4_000));
    steps.push(AllOff);
    steps.push(Render(6_000));
    steps
}

#[test]
fn renders_the_old_synth_sample_for_sample_with_constant_gain_and_sustain() {
    let steps = constant_performance();
    let old = render_old(&steps);
    assert!(old.iter().any(|s| s.abs() > 0.1), "the performance sounds");
    for size in [1, 7, 128, 1000, 2048] {
        let new = render_new(&steps, size);
        assert_eq!(new.len(), old.len());
        let first = old.iter().zip(&new).position(|(a, b)| a != b);
        assert_eq!(
            first, None,
            "blocks of {size}: first difference at frame {first:?}"
        );
    }
}

/// Frames after which the old gain smoother (one-pole, time constant 10 ms) has stopped
/// moving. In f32 it stalls a few 1e-6 short of its target (its step falls below half an
/// ulp before its 1e-6 snap applies), so the renders keep a difference of that order, about
/// -100 dB: [`AFTER_GAIN_MOVE`].
const ONE_POLE_SETTLED: usize = 9_600;

/// Largest difference left once the old one-pole has stalled (above).
const AFTER_GAIN_MOVE: f32 = 1e-5;

#[test]
fn gain_moves_differ_only_inside_the_smoothing_and_by_less_than_the_move() {
    use Step::*;
    let moves = [(0.8f32, 0.2f32), (0.2, 1.0), (1.0, 0.0), (0.0, 0.5)];
    let mut steps = vec![On(48, 100), On(55, 100), On(60, 100), Render(12_000)];
    for (_, to) in moves {
        steps.push(Set(Param::Gain, to));
        steps.push(Render(12_000));
    }
    steps.push(AllOff);
    steps.push(Render(20_000));
    let old = render_old(&steps);
    let new = render_new(&steps, 128);
    for (i, (from, to)) in moves.iter().enumerate() {
        let start = 12_000 * (i + 1);
        let window = &new[start..start + ONE_POLE_SETTLED];
        let reference = &old[start..start + ONE_POLE_SETTLED];
        let during = max_diff(window, reference);
        assert!(
            during <= (to - from).abs() + 1e-6,
            "move {i}: {during} during the ramp"
        );
        let after = max_diff(
            &new[start + ONE_POLE_SETTLED..start + 12_000],
            &old[start + ONE_POLE_SETTLED..start + 12_000],
        );
        assert!(
            after <= AFTER_GAIN_MOVE,
            "move {i}: {after} after the ramps"
        );
    }
}

#[test]
fn sustain_moves_on_held_notes_differ_only_inside_the_smoothing() {
    use Step::*;
    // Held notes past their decay (0.2 s): every voice is in its sustain stage.
    let moves = [0.2f32, 0.9, 0.0, 0.6];
    let mut steps = vec![On(57, 110), On(64, 110), Render(24_000)];
    for to in moves {
        steps.push(Set(Param::Sustain, to));
        steps.push(Render(4_800));
    }
    let old = render_old(&steps);
    let new = render_new(&steps, 64);
    assert_eq!(old[..24_000], new[..24_000]);
    let mut from = 0.7f32;
    for (i, to) in moves.iter().enumerate() {
        let start = 24_000 + 4_800 * i;
        // 5 ms: both curves have reached the new level.
        let ramp = 240;
        let during = max_diff(&new[start..start + ramp], &old[start..start + ramp]);
        assert!(
            during <= (to - from).abs() * 0.5,
            "move {i}: {during} during the ramp"
        );
        let after = max_diff(
            &new[start + ramp..start + 4_800],
            &old[start + ramp..start + 4_800],
        );
        assert!(after <= 1e-6, "move {i}: {after} after the ramp");
        from = *to;
    }
}

/// A sustain move while a voice decays changes the decay's slope for 5 ms: the level the
/// voice reaches its sustain from differs slightly, and the decay ends a few frames apart.
/// Bounded here, documented in the device (module doc) and the pull request.
#[test]
fn a_sustain_move_during_the_decay_leaves_a_small_lasting_difference() {
    use Step::*;
    let steps = [
        On(60, 110),
        Render(1_200), // past the 5 ms attack, in the 0.2 s decay
        Set(Param::Sustain, 0.2),
        Render(48_000),
    ];
    let old = render_old(&steps);
    let new = render_new(&steps, 128);
    let diff = max_diff(&old, &new);
    assert!(diff > 0.0, "the curves differ");
    assert!(diff < 2e-3, "{diff}");
    // Once both voices sit at the new sustain, the renders agree again.
    let late = max_diff(&old[24_000..], &new[24_000..]);
    assert!(late <= 1e-6, "{late}");
}

// ---------------------------------------------------------------- the declared smoothing

/// The device's signal before gain: `atanh(out)` of a render at gain 1 (sum of the voices).
fn sum_at_unity(events: &[(usize, Event)], total: usize) -> Vec<f32> {
    let mut d = Synth::new(SR);
    Device::set_param(&mut d, pid(Param::Gain), 1.0);
    Device::reset(&mut d);
    timeline(&mut d, total, 256, events)
        .iter()
        .map(|s| s.atanh())
        .collect()
}

#[test]
fn a_gain_change_is_a_linear_ramp_that_lands_exactly_after_ten_ms() {
    let notes = [(0, param(0, Param::Waveform, 0.0)), (0, on(0, 1, 60, 0.5))];
    let total = 8_000;
    let sum = sum_at_unity(&notes, total);
    let n = OSC_SYNTH.params[Param::Gain.id() as usize].smoothing_frames(SR) as usize;
    assert_eq!(n, 480);
    for k in [1_000usize, 1_001, 1_123, 1_279] {
        let mut d = Synth::new(SR);
        Device::set_param(&mut d, pid(Param::Gain), 1.0);
        Device::reset(&mut d);
        let mut events = notes.to_vec();
        events.push((k, param(0, Param::Gain, 0.5)));
        // A new value during the ramp: a new ramp from where the value is.
        events.push((k + 100, param(0, Param::Gain, 0.0)));
        let out = timeline(&mut d, total, 128, &events);
        let at_restart = 1.0 + (0.5 - 1.0) * 100.0 / 480.0;
        for (t, s) in out.iter().enumerate() {
            let g = if t < k {
                1.0
            } else if t < k + 100 {
                1.0 + (0.5 - 1.0) * (t - k) as f32 / 480.0
            } else {
                let i = (t - k - 100).min(480) as f32;
                at_restart + (0.0 - at_restart) * i / 480.0
            };
            let expected = (sum[t] * g).tanh();
            assert!(
                (s - expected).abs() <= 1e-6,
                "k {k}, frame {t}: {s} vs {expected}"
            );
        }
        assert!(out[k + 100 + 480..].iter().all(|s| *s == 0.0), "lands on 0");
        assert!(out[k + 100 + 479] != 0.0, "not before");
    }
}

#[test]
fn a_sustain_change_is_a_linear_ramp_that_lands_exactly_after_five_ms() {
    let notes = [
        (0, param(0, Param::Waveform, 0.0)),
        (0, param(0, Param::Gain, 1.0)),
        (0, param(0, Param::Decay, 0.01)),
        (0, on(0, 1, 60, 0.5)),
    ];
    let total = 12_000;
    let mut held = Synth::new(SR);
    let reference = timeline(&mut held, total, 256, &notes);
    let n = OSC_SYNTH.params[Param::Sustain.id() as usize].smoothing_frames(SR) as usize;
    assert_eq!(n, 240);
    let k = 6_007; // the voice sits at sustain 0.7
    let mut d = Synth::new(SR);
    let mut events = notes.to_vec();
    events.push((k, param(0, Param::Sustain, 0.3)));
    let out = timeline(&mut d, total, 100, &events);
    assert_eq!(out[..k], reference[..k]);
    for t in k..total {
        // The voice's level is the sustain: the output is the reference scaled by it.
        let s = 0.7 + (0.3 - 0.7) * ((t - k).min(n) as f32) / n as f32;
        let expected = (reference[t].atanh() * s / 0.7).tanh();
        assert!(
            (out[t] - expected).abs() <= 1e-6,
            "frame {t}: {} vs {expected}",
            out[t]
        );
    }
}

// ---------------------------------------------------------------- events

#[test]
fn a_note_starts_at_its_exact_offset_whatever_the_block_size() {
    let events = [
        (0, param(0, Param::Waveform, 3.0)), // triangle: nonzero from the first frame
        (1_234, on(0, 9, 69, 1.0)),
        (3_001, off(0, 9, 69)),
        (3_500, param(0, Param::Gain, 0.3)),
    ];
    let reference = timeline(&mut Synth::new(SR), 24_000, 2048, &events);
    assert!(reference[..1_234].iter().all(|s| *s == 0.0));
    assert!(reference[1_234] != 0.0);
    assert_clean(&reference);
    for size in [1, 7, 128, 999] {
        let out = timeline(&mut Synth::new(SR), 24_000, size, &events);
        assert_eq!(out, reference, "blocks of {size}");
    }
}

#[test]
fn notes_are_their_voice_ids() {
    let mut s = Synth::new(SR);
    // Two notes on one key, two voice ids.
    block(&mut s, 64, &[on(0, 10, 60, 1.0), on(10, 11, 60, 1.0)]);
    assert_eq!(s.active_voices(), 2);
    // A note-off names its voice id; its key is informational.
    block(&mut s, 64, &[off(5, 10, 99)]);
    block(&mut s, 48_000, &[]);
    assert_eq!(s.active_voices(), 1);
    assert!(s.is_held(60));
    // A note-on of a sounding voice id retriggers it.
    block(&mut s, 64, &[on(0, 11, 62, 1.0)]);
    block(&mut s, 4_800, &[]);
    assert_eq!(s.active_voices(), 1);
    assert!(s.is_held(62) && !s.is_held(60));
    // A tap shorter than a block: on and off at one offset ends at once, never stuck.
    block(&mut s, 64, &[on(3, 12, 64, 1.0), off(3, 12, 64)]);
    block(&mut s, 48_000, &[]);
    assert_eq!(s.active_voices(), 1);
    // An off before the on of a stolen voice's queued note drops it.
    block(&mut s, 64, &[Event::AllNotesOff { offset: 0 }]);
    block(&mut s, 48_000, &[]);
    assert_eq!(s.active_voices(), 0);
}

#[test]
fn a_note_off_drops_a_note_waiting_for_its_voice() {
    let mut s = Synth::new(SR);
    let held: Vec<Event> = (0..16).map(|i| on(0, 100 + i, 40 + i as u8, 1.0)).collect();
    block(&mut s, 64, &held);
    // The seventeenth note steals a voice and waits for its fade; its off drops it.
    block(&mut s, 64, &[on(0, 7, 80, 1.0), off(1, 7, 80)]);
    let release: Vec<Event> = (0..16).map(|i| off(0, 100 + i, 40 + i as u8)).collect();
    block(&mut s, 64, &release);
    block(&mut s, 48_000, &[]);
    assert_eq!(s.active_voices(), 0);
    assert!(!s.is_held(80));
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
fn fine_tuning_moves_the_pitch_by_cents() {
    for (key, cents, hz) in [
        (69u8, 0.0f32, 440.0f32),
        (69, 50.0, 440.0 * 2f32.powf(0.5 / 12.0)),
        (69, -100.0, 440.0 * 2f32.powf(-1.0 / 12.0)),
        (57, 5_000.0, 440.0), // clamped to +1200 cents
        (69, f32::NAN, 440.0),
    ] {
        let mut s = Synth::new(SR);
        let events = [
            param(0, Param::Waveform, 0.0),
            Event::NoteOn {
                offset: 0,
                voice: voice(1),
                key,
                velocity: 0.8,
                tune_cents: cents,
            },
        ];
        block(&mut s, 4_800, &events);
        let out = block(&mut s, 48_000, &[]);
        let f = frequency(&out);
        assert!((f - hz).abs() < 0.5, "{key} {cents}: {f} Hz, expected {hz}");
    }
}

// ---------------------------------------------------------------- set_param and reset

#[test]
fn set_param_is_a_value_at_offset_0_of_the_next_block() {
    let start = [param(0, Param::Waveform, 0.0), on(0, 1, 60, 1.0)];
    let mut a = Synth::new(SR);
    let mut b = Synth::new(SR);
    let mut out_a = block(&mut a, 2_000, &start);
    let mut out_b = block(&mut b, 2_000, &start);
    for (p, v) in [
        (Param::Gain, 0.2),
        (Param::Sustain, 0.1),
        (Param::Waveform, 2.0),
    ] {
        Device::set_param(&mut a, pid(p), v);
        out_a.extend(block(&mut a, 300, &[]));
        out_b.extend(block(&mut b, 300, &[param(0, p, v)]));
    }
    assert_eq!(out_a, out_b);
}

#[test]
fn reset_silences_at_once_and_snaps_the_ramps() {
    let mut s = Synth::new(SR);
    block(&mut s, 2_000, &[on(0, 1, 60, 1.0), on(0, 2, 64, 1.0)]);
    Device::set_param(&mut s, pid(Param::Gain), 0.2);
    Device::set_param(&mut s, pid(Param::Sustain), 0.4);
    Device::reset(&mut s);
    assert_eq!(s.active_voices(), 0);
    assert!(block(&mut s, 512, &[]).iter().all(|x| *x == 0.0));
    assert_eq!((s.param(Param::Gain), s.param(Param::Sustain)), (0.2, 0.4));
    // The next note plays as if gain and sustain had always been there.
    let mut fresh = Synth::new(SR);
    Device::set_param(&mut fresh, pid(Param::Gain), 0.2);
    Device::set_param(&mut fresh, pid(Param::Sustain), 0.4);
    Device::reset(&mut fresh);
    let note = [on(0, 1, 60, 1.0)];
    assert_eq!(
        block(&mut s, 24_000, &note),
        block(&mut fresh, 24_000, &note)
    );
}

#[test]
fn values_are_sanitized_by_the_descriptor() {
    let mut s = Synth::new(SR);
    for (p, raw, kept) in [
        (Param::Waveform, 2.6, 3.0),
        (Param::Waveform, -4.0, 0.0),
        (Param::Waveform, 1.49, 1.0),
        (Param::Attack, 0.0, 0.002),
        (Param::Release, 99.0, 30.0),
        (Param::Sustain, 1.5, 1.0),
        (Param::Gain, -1.0, 0.0),
    ] {
        Device::set_param(&mut s, pid(p), raw);
        assert_eq!(s.param(p), kept, "{p:?} {raw}");
        Device::set_param(&mut s, pid(p), f32::NAN);
        assert_eq!(s.param(p), kept, "{p:?}: NaN ignored");
    }
    let before = s.clone();
    Device::set_param(&mut s, ParamId(6), 1.0);
    Device::set_param(&mut s, ParamId(u32::MAX), 1.0);
    assert_eq!(
        format!("{before:?}"),
        format!("{s:?}"),
        "unknown ids ignored"
    );
}

// ---------------------------------------------------------------- the registry

#[test]
fn the_registry_builds_each_of_its_devices() {
    assert_eq!(REGISTRY.len(), builtin_descriptors().count());
    for (i, line) in REGISTRY.iter().enumerate() {
        let d = line.descriptor;
        assert_eq!(d.validate(), Ok(()), "{}", d.kind);
        assert!(
            REGISTRY[..i]
                .iter()
                .all(|o| o.descriptor.kind_id != d.kind_id)
        );
        assert_eq!(builtin(d.kind_id).map(|b| b.kind), Some(d.kind));
        let device =
            builtin_factory(d.kind_id).expect("resolved")(&DeviceSetup::default()).expect("built");
        assert_eq!(device.descriptor().kind, d.kind);
        assert!(d.memory_bytes >= size_of_val(device.as_ref()) as u64);
    }
    assert!(builtin_factory(DeviceKind(0)).is_none());
    assert!(builtin_factory(DeviceKind(999)).is_none());
    let synth = builtin(OSC_SYNTH.kind_id).unwrap();
    assert_eq!((synth.kind, synth.name), ("faulk.osc_synth", "Oscillator"));
    assert_eq!(synth.role, DeviceRole::Instrument);
}

#[test]
fn every_device_of_the_registry_passes_the_conformance_run() {
    for line in REGISTRY {
        for rate in [8_000.0f32, 44_100.0, 48_000.0, 192_000.0] {
            let setup = DeviceSetup {
                sample_rate: rate,
                ..DeviceSetup::default()
            };
            for seed in 1..=4u64 {
                let mut device = (line.factory)(&setup).unwrap();
                let mut out = EventBuffer::try_with_capacity(256).unwrap();
                let result =
                    assert_no_alloc(|| conformance(device.as_mut(), rate, seed, 300, &mut out));
                assert_eq!(
                    result,
                    Ok(()),
                    "{} at {rate} Hz, seed {seed}",
                    line.descriptor.kind
                );
            }
        }
    }
}

// ---------------------------------------------------------------- listening

/// Writes a short listening render of the synth when `FAULK_RENDER_DIR` is set (16-bit
/// mono WAV, 48 kHz): a chord on each waveform, gain and sustain moves, a release.
#[test]
fn listening_render() {
    let Some(dir) = std::env::var_os("FAULK_RENDER_DIR") else {
        return;
    };
    let mut events = Vec::new();
    let s = |sec: f32| (sec * SR) as usize;
    for (i, w) in [1.0, 0.0, 2.0, 3.0].iter().enumerate() {
        let t = i as f32 * 1.5;
        events.push((s(t), param(0, Param::Waveform, *w)));
        for (j, key) in [48u8, 55, 60, 64].iter().enumerate() {
            let id = (i * 10 + j + 1) as u32;
            events.push((s(t) + j * 2_400, on(0, id, *key, 0.8)));
            events.push((s(t + 1.2), off(0, id, *key)));
        }
    }
    events.push((s(0.6), param(0, Param::Gain, 0.3)));
    events.push((s(0.9), param(0, Param::Gain, 0.8)));
    events.push((s(3.4), param(0, Param::Sustain, 0.2)));
    events.push((s(3.8), param(0, Param::Sustain, 0.9)));
    events.sort_by_key(|(at, e)| (*at, e.rank()));
    let out = timeline(&mut Synth::new(SR), s(6.5), 128, &events);
    assert_clean(&out);
    let mut wav = Vec::new();
    let data = (out.len() * 2) as u32;
    wav.extend(b"RIFF");
    wav.extend((36 + data).to_le_bytes());
    wav.extend(b"WAVEfmt ");
    wav.extend(16u32.to_le_bytes());
    wav.extend(1u16.to_le_bytes());
    wav.extend(1u16.to_le_bytes());
    wav.extend((SR as u32).to_le_bytes());
    wav.extend((SR as u32 * 2).to_le_bytes());
    wav.extend(2u16.to_le_bytes());
    wav.extend(16u16.to_le_bytes());
    wav.extend(b"data");
    wav.extend(data.to_le_bytes());
    for x in out {
        wav.extend(((x.clamp(-1.0, 1.0) * 32_767.0) as i16).to_le_bytes());
    }
    let path = std::path::Path::new(&dir).join("osc_synth-dv1.wav");
    std::fs::write(&path, wav).unwrap();
    println!("wrote {}", path.display());
}
