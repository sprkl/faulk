//! The synthesized drums, `faulk.drums` (DV-6): offline renders through the device contract,
//! every `process` under the allocation detector. Asserts on the signal for each sound
//! (level, spectrum, decay, silence once it ended, no NaN, no denormal), on the mapping of
//! the notes, on retriggers, on events at their exact frame and on the parameters. The
//! contract's conformance run on every device of the registry is in `osc_synth.rs`; the
//! native/WASM parity is `crates/engine-wasm/tests/drums_parity.rs`.

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use engine::device::{
    AudioIo, BlockCtx, Device, DeviceRole, Event, EventBuffer, Transport, Unit, VoiceId, builtin,
};
use engine::devices::drums::{self, DESCRIPTOR, Drums};
use engine::msg::ParamId;
use test_support::{Render, Spectrum};

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

const SR: f32 = 48_000.0;

fn on(offset: u32, key: u8, velocity: f32) -> Event {
    Event::NoteOn {
        offset,
        voice: VoiceId::new(u32::from(key) + 1).unwrap(),
        key,
        velocity,
        tune_cents: 0.0,
    }
}

fn set(offset: u32, id: u32, value: f32) -> Event {
    Event::ParamValue {
        offset,
        id: ParamId(id),
        value,
    }
}

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
    assert_eq!(left, right, "the drums are mono");
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

fn render(events: &[(usize, Event)], seconds: f32) -> Render {
    let signal = timeline(&mut Drums::new(SR), (seconds * SR) as usize, 128, events);
    let render = Render::new(SR, signal.clone(), signal);
    render.assert_clean();
    render
}

/// One hit of `key` at velocity 1 with the given parameter values set first.
fn hit(key: u8, params: &[(u32, f32)], seconds: f32) -> Render {
    let mut events: Vec<(usize, Event)> =
        params.iter().map(|(id, v)| (0, set(0, *id, *v))).collect();
    events.push((0, on(0, key, 1.0)));
    render(&events, seconds)
}

fn energy(spectrum: &Spectrum, band: std::ops::Range<f32>) -> f32 {
    spectrum
        .magnitudes
        .iter()
        .enumerate()
        .filter(|(i, _)| band.contains(&(*i as f32 * spectrum.bin_hz)))
        .map(|(_, m)| m * m)
        .sum()
}

fn share(spectrum: &Spectrum, band: std::ops::Range<f32>) -> f32 {
    energy(spectrum, band) / energy(spectrum, 0.0..SR / 2.0)
}

// ---------------------------------------------------------------- each sound

/// Every sound: audible and under full scale at once, falls by 40 dB within its decay
/// time, and is exactly silent after 1.4 times that time.
fn check_decay(key: u8, decay_id: u32, extra: f32) {
    let decay = DESCRIPTOR.param(ParamId(decay_id)).unwrap().default as f32;
    let seconds = extra + decay * 1.4 + 0.1;
    let r = hit(key, &[], seconds);
    r.assert_audible(0.0..0.02, 0.05);
    let early = r.rms_in(0.0..0.02);
    let late = r.rms_in(extra + decay..extra + decay + 0.02);
    assert!(late < early * 0.01, "key {key}: early {early}, late {late}");
    r.assert_silent(extra + decay * 1.4 + 0.02..seconds);
}

#[test]
fn the_kick_is_a_low_sine_that_falls_and_ends() {
    check_decay(drums::KICK_KEY, drums::KICK_DECAY, 0.0);
    let r = hit(drums::KICK_KEY, &[], 0.6);
    let s = r.spectrum(0.05..0.3);
    assert!(s.dominant_hz() < 100.0, "dominant {}", s.dominant_hz());
    assert!(
        share(&s, 0.0..300.0) > 0.9,
        "low share {}",
        share(&s, 0.0..300.0)
    );
}

#[test]
fn the_kick_follows_its_tuning() {
    let tuned = |hz: f32| {
        hit(
            drums::KICK_KEY,
            &[(drums::KICK_TUNE, hz), (drums::KICK_PUNCH, 0.0)],
            0.6,
        )
        .spectrum(0.05..0.3)
        .dominant_hz()
    };
    let (a, b) = (tuned(40.0), tuned(100.0));
    assert!((a - 40.0).abs() < 4.0, "{a}");
    assert!((b - 100.0).abs() < 4.0, "{b}");
}

#[test]
fn the_snare_is_a_tone_and_highpassed_noise() {
    check_decay(drums::SNARE_KEY, drums::SNARE_DECAY, 0.0);
    let r = hit(drums::SNARE_KEY, &[], 0.5);
    let s = r.spectrum(0.0..0.1);
    assert!(
        share(&s, 2_000.0..SR / 2.0) > 0.3,
        "{}",
        share(&s, 2_000.0..SR / 2.0)
    );
    assert!(s.peak_in(150.0..250.0) > 0.01, "the tone is there");
    // The snap is the noise: without it only the tone is left.
    let tone = hit(drums::SNARE_KEY, &[(drums::SNARE_SNAP, 0.0)], 0.5).spectrum(0.0..0.1);
    assert!(share(&tone, 0.0..400.0) > 0.9);
}

#[test]
fn the_clap_is_bandpassed_noise_in_bursts() {
    check_decay(drums::CLAP_KEY, drums::CLAP_DECAY, 0.033);
    let r = hit(drums::CLAP_KEY, &[], 0.6);
    let s = r.spectrum(0.0..0.1);
    assert!(
        share(&s, 800.0..3_000.0) > 0.6,
        "{}",
        share(&s, 800.0..3_000.0)
    );
    // The second burst starts at 11 ms: the level is higher just after than just before.
    assert!(
        r.rms_in(0.0112..0.0132) > r.rms_in(0.0092..0.0108) * 1.5,
        "a second burst starts"
    );
}

#[test]
fn the_hats_are_bright_and_the_open_one_rings_longer() {
    check_decay(drums::CLOSED_HAT_KEY, drums::HAT_DECAY, 0.0);
    check_decay(drums::OPEN_HAT_KEY, drums::HAT_OPEN_DECAY, 0.0);
    let r = hit(drums::CLOSED_HAT_KEY, &[], 0.3);
    let s = r.spectrum(0.0..0.03);
    assert!(
        share(&s, 3_000.0..SR / 2.0) > 0.9,
        "{}",
        share(&s, 3_000.0..SR / 2.0)
    );
    let closed = hit(drums::CLOSED_HAT_KEY, &[], 0.5).rms_in(0.2..0.22);
    let open = hit(drums::OPEN_HAT_KEY, &[], 0.5).rms_in(0.2..0.22);
    assert!(open > closed * 10.0, "closed {closed}, open {open}");
}

#[test]
fn a_closed_hat_chokes_the_open_one() {
    let events = [
        (0, on(0, drums::OPEN_HAT_KEY, 1.0)),
        (4_800, on(0, drums::CLOSED_HAT_KEY, 1.0)),
    ];
    let r = render(&events, 0.6);
    let open_alone = hit(drums::OPEN_HAT_KEY, &[], 0.6);
    assert!(r.rms_in(0.3..0.32) < open_alone.rms_in(0.3..0.32) * 0.05);
}

#[test]
fn every_sound_has_its_own_note_and_the_others_are_ignored() {
    let sounding = [36u8, 38, 39, 42, 46];
    for key in 0..=127u8 {
        let r = hit(key, &[], 0.05);
        if sounding.contains(&key) {
            r.assert_audible(0.0..0.05, 0.05);
        } else {
            r.assert_silent(0.0..0.05);
        }
    }
}

#[test]
fn the_velocity_scales_the_hit_and_zero_is_no_hit() {
    let at = |velocity: f32| render(&[(0, on(0, drums::SNARE_KEY, velocity))], 0.1);
    let (soft, loud) = (at(0.25).rms(), at(1.0).rms());
    assert!(
        loud > soft * 2.5 && loud < soft * 5.0,
        "soft {soft}, loud {loud}"
    );
    for velocity in [0.0, -1.0, f32::NAN, f32::INFINITY * 0.0] {
        at(velocity).assert_silent(0.0..0.1);
    }
    // Above 1 is clamped to 1.
    assert_eq!(at(7.0), at(1.0));
}

#[test]
fn a_note_off_is_ignored_and_all_notes_off_fades_every_sound_out() {
    let off = Event::NoteOff {
        offset: 0,
        voice: VoiceId::new(37).unwrap(),
        key: 36,
    };
    let held = render(&[(0, on(0, 36, 1.0)), (2_400, off)], 0.3);
    let plain = render(&[(0, on(0, 36, 1.0))], 0.3);
    assert_eq!(held, plain, "one-shot");

    let all = Event::AllNotesOff { offset: 0 };
    let events = [
        (0, on(0, 36, 1.0)),
        (0, on(0, 38, 1.0)),
        (0, on(0, 46, 1.0)),
        (2_400, all),
    ];
    let r = render(&events, 0.3);
    r.assert_audible(0.0..0.05, 0.1);
    r.assert_silent(0.05 + 0.004..0.3);
}

// ---------------------------------------------------------------- retrigger and timing

#[test]
fn a_hit_on_a_ringing_sound_fades_then_restarts_without_a_click() {
    let events = [(0, on(0, 36, 1.0)), (2_400, on(0, 36, 1.0))];
    let r = render(&events, 0.4);
    // The restart is after the 3 ms fade; the new hit starts from phase 0, a zero.
    let restart = 2_400 + (0.003 * SR) as usize;
    let jump = r.left[2_300..restart + 200]
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0f32, f32::max);
    assert!(jump < 0.12, "largest step {jump}");
    let before = r.rms_in(0.0..0.05);
    let after = r.rms_in(restart as f32 / SR..restart as f32 / SR + 0.05);
    assert!(
        after > before * 0.5,
        "the second hit is as loud as the first"
    );
    // Hits during the fade: the last one wins.
    let burst = [
        (0, on(0, 36, 1.0)),
        (2_400, on(0, 36, 0.3)),
        (2_450, on(0, 36, 1.0)),
    ];
    let last = render(&burst, 0.3);
    let alone = render(&[(0, on(0, 36, 1.0)), (2_400, on(0, 36, 1.0))], 0.3);
    last.assert_close(&alone, 0.0);
}

#[test]
fn a_hit_starts_at_its_exact_frame_whatever_the_block_size() {
    let events = [(1_000, on(0, 38, 1.0)), (6_000, on(0, 42, 1.0))];
    let reference = timeline(&mut Drums::new(SR), 12_000, 128, &events);
    assert!(reference[..1_000].iter().all(|s| *s == 0.0));
    assert!(reference[1_000..1_100].iter().any(|s| s.abs() > 1e-4));
    for size in [1usize, 7, 37, 512, 4_096] {
        let other = timeline(&mut Drums::new(SR), 12_000, size, &events);
        assert_eq!(other, reference, "blocks of {size}");
    }
}

#[test]
fn a_render_is_the_same_every_time_and_after_a_reset() {
    let events = [
        (0, on(0, 38, 1.0)),
        (3_000, on(0, 39, 0.8)),
        (6_000, on(0, 42, 0.8)),
    ];
    let mut device = Drums::new(SR);
    let first = timeline(&mut device, 20_000, 256, &events);
    device.reset();
    let again = timeline(&mut device, 20_000, 256, &events);
    assert_eq!(first, again);
    assert_eq!(first, timeline(&mut Drums::new(SR), 20_000, 256, &events));
}

#[test]
fn a_busy_pattern_stays_clean_at_every_rate_under_the_allocation_detector() {
    for rate in [8_000.0f32, 44_100.0, 48_000.0, 192_000.0] {
        let mut device = Drums::new(rate);
        let mut out = Vec::new();
        let mut unused = EventBuffer::try_with_capacity(1).unwrap();
        for step in 0..400u32 {
            let key = [36u8, 38, 39, 42, 46, 42, 38, 200][(step % 8) as usize];
            let value = [0.0, 0.5, 1.0, 1e9, -1e9, f32::NAN][(step % 6) as usize];
            let events = [set(0, step % 18, value), on(10, key, 1.0), on(20, key, 0.6)];
            let ctx = BlockCtx {
                sample_rate: rate,
                frames: 512,
                transport: Transport::default(),
            };
            let (mut l, mut r) = (vec![0.0f32; 512], vec![0.0f32; 512]);
            assert_no_alloc(|| {
                let mut io = AudioIo {
                    left: &mut l,
                    right: &mut r,
                    events_out: &mut unused,
                };
                device.process(&ctx, &events, &mut io);
            });
            out.extend(l);
        }
        Render::new(rate, out.clone(), out).assert_clean();
    }
}

// ---------------------------------------------------------------- parameters

#[test]
fn the_descriptor_is_valid_and_the_defaults_are_the_device_values() {
    assert_eq!(DESCRIPTOR.validate(), Ok(()));
    assert_eq!((DESCRIPTOR.kind, DESCRIPTOR.name), ("faulk.drums", "Drums"));
    assert_eq!(DESCRIPTOR.role, DeviceRole::Instrument);
    assert_eq!(
        builtin(DESCRIPTOR.kind_id).map(|b| b.kind),
        Some("faulk.drums")
    );
    let device = Drums::new(SR);
    for p in DESCRIPTOR.params {
        assert_eq!(device.param(p.id.0), p.default as f32, "{}", p.key);
    }
    // Ids are the positions: the constants and the descriptor agree.
    for (i, p) in DESCRIPTOR.params.iter().enumerate() {
        assert_eq!(p.id.0 as usize, i);
    }
    assert_eq!(DESCRIPTOR.params.len(), 16);
}

#[test]
fn each_sound_has_its_own_parameters_with_declared_smoothing_on_the_levels() {
    for p in DESCRIPTOR.params {
        let level = p.key.ends_with("_level") || p.key == "gain";
        assert_eq!(p.unit == Unit::Gain, level, "{}", p.key);
        assert_eq!(
            p.smoothing_seconds,
            if level { 0.01 } else { 0.0 },
            "{}",
            p.key
        );
        assert_eq!(p.step, 0.0);
    }
    for sound in ["kick", "snare", "clap", "hat"] {
        let mine = DESCRIPTOR
            .params
            .iter()
            .filter(|p| p.key.starts_with(sound))
            .count();
        assert!(mine >= 3, "{sound} has {mine} parameters");
    }
}

#[test]
fn values_are_sanitized_by_the_descriptor() {
    let mut device = Drums::new(SR);
    device.set_param(ParamId(drums::KICK_TUNE), 1e9);
    assert_eq!(device.param(drums::KICK_TUNE), 120.0);
    device.set_param(ParamId(drums::KICK_TUNE), -5.0);
    assert_eq!(device.param(drums::KICK_TUNE), 30.0);
    device.set_param(ParamId(drums::KICK_TUNE), f32::NAN);
    assert_eq!(device.param(drums::KICK_TUNE), 30.0);
    device.set_param(ParamId(drums::GAIN), 3.0);
    assert_eq!(device.param(drums::GAIN), 1.0);
    for id in [16, 17, 1_000, u32::MAX] {
        device.set_param(ParamId(id), 0.5);
    }
    assert_eq!(device.param(16), 0.0);
}

#[test]
fn a_level_is_a_linear_ramp_that_lands_after_ten_ms_and_every_level_scales_its_sound() {
    // A quiet kick (the soft clip is then linear to better than 0.1 %), the level to 0 at
    // frame 1000: against the same render with the level held, the ratio is the ramp.
    let hold = [
        (0, set(0, drums::KICK_LEVEL, 1.0)),
        (0, set(0, drums::GAIN, 1.0)),
        (0, on(0, 36, 0.05)),
    ];
    let mut ramped = hold.to_vec();
    ramped.push((1_000, set(0, drums::KICK_LEVEL, 0.0)));
    let (held, ramp) = (render(&hold, 0.2), render(&ramped, 0.2));
    let n = (0.01 * SR) as usize;
    assert_eq!(ramp.left[..1_000], held.left[..1_000]);
    for i in 0..n {
        let (a, b) = (held.left[1_000 + i], ramp.left[1_000 + i]);
        assert!(a.abs() > 1e-4, "frame {i}");
        let expected = 1.0 - i as f32 / n as f32;
        assert!(
            (b / a - expected).abs() < 2e-3,
            "frame {i}: {} vs {expected}",
            b / a
        );
    }
    assert!(
        ramp.left[1_000 + n..].iter().all(|s| *s == 0.0),
        "the level reached 0"
    );
    for (id, key) in [
        (drums::KICK_LEVEL, 36u8),
        (drums::SNARE_LEVEL, 38),
        (drums::CLAP_LEVEL, 39),
        (drums::HAT_LEVEL, 42),
        (drums::HAT_LEVEL, 46),
        (drums::GAIN, 36),
    ] {
        // Set at frame 0 of the block: the ramp lands within 10 ms.
        hit(key, &[(id, 0.0)], 0.1).assert_silent(0.011..0.1);
    }
}

#[test]
fn the_decay_parameters_set_the_length_of_each_sound() {
    for (key, id) in [
        (36u8, drums::KICK_DECAY),
        (38, drums::SNARE_DECAY),
        (39, drums::CLAP_DECAY),
        (42, drums::HAT_DECAY),
        (46, drums::HAT_OPEN_DECAY),
    ] {
        let d = DESCRIPTOR.param(ParamId(id)).unwrap();
        let short = hit(key, &[(id, d.min as f32)], 1.0).rms_in(0.25..0.3);
        let long = hit(key, &[(id, d.max as f32)], 1.0).rms_in(0.25..0.3);
        assert!(
            long > short * 10.0 || (short == 0.0 && long > 0.0),
            "key {key}: {short} vs {long}"
        );
    }
}

#[test]
fn reset_silences_at_once_and_keeps_the_parameters() {
    let mut device = Drums::new(SR);
    device.set_param(ParamId(drums::KICK_TUNE), 80.0);
    block(&mut device, 128, &[on(0, 36, 1.0), on(0, 46, 1.0)]);
    assert_eq!(device.active_voices(), 2);
    device.reset();
    assert_eq!(device.active_voices(), 0);
    assert!(block(&mut device, 128, &[]).iter().all(|s| *s == 0.0));
    assert_eq!(device.param(drums::KICK_TUNE), 80.0);
}

// ---------------------------------------------------------------- listening

/// Writes the listening renders when `FAULK_RENDER_DIR` is set: one file per sound, with the
/// default parameters, then a simple pattern. 32-bit float stereo WAV files.
#[test]
fn listening_renders() {
    let Some(dir) = std::env::var_os("FAULK_RENDER_DIR") else {
        return;
    };
    let dir = std::path::Path::new(&dir);
    std::fs::create_dir_all(dir).unwrap();
    let write = |name: &str, r: &Render| {
        r.assert_clean();
        let path = dir.join(format!("{name}.wav"));
        std::fs::write(&path, r.wav()).unwrap();
        println!("wrote {}", path.display());
    };
    for (name, key, seconds) in [
        ("drums-kick", 36u8, 1.0f32),
        ("drums-snare", 38, 0.8),
        ("drums-clap", 39, 0.8),
        ("drums-closed-hat", 42, 0.5),
        ("drums-open-hat", 46, 1.2),
    ] {
        write(name, &hit(key, &[], seconds));
    }
    // Two bars at 100 BPM, sixteenth notes: kick on 1 and 3 (and an extra), snare and clap on
    // 2 and 4, closed hats on the eighths, an open hat at the end of each bar.
    let step = (SR * 60.0 / 100.0 / 4.0) as usize;
    let mut events = Vec::new();
    for bar in 0..2usize {
        for s in 0..16usize {
            let at = (bar * 16 + s) * step;
            let mut play = |key: u8, v: f32| events.push((at, on(0, key, v)));
            if matches!(s, 0 | 8) || (s == 10 && bar == 1) {
                play(36, 1.0);
            }
            if matches!(s, 4 | 12) {
                play(38, 0.9);
                play(39, 0.6);
            }
            if s % 2 == 0 && s != 14 {
                play(42, if s % 4 == 0 { 0.8 } else { 0.5 });
            }
            if s == 14 {
                play(46, 0.7);
            }
        }
    }
    events.sort_by_key(|(at, e)| (*at, e.rank()));
    write(
        "drums-pattern",
        &render(&events, 2.0 * 16.0 * step as f32 / SR + 0.8),
    );
}
