//! The spectrum analysis of the visual feedback (#156, `engine::analysis`):
//!
//! - the FFT against a direct DFT;
//! - a sine lands in its band at 0 dBFS, at every window length;
//! - the bands above Nyquist read the floor;
//! - Contour's window is its output, and a bell's lift shows in the bands;
//! - the whole path through the host (window region, `Prepare::flush`, spectrum region, report
//!   v10) with the render and the report under the allocation detector;
//! - the round robin over more devices than a report carries, also when the count shrinks;
//! - a reset or a bypass hides the window;
//! - the seqlocks refuse an odd or unchanged region.

use assert_no_alloc::assert_no_alloc;
use engine::OSC_SYNTH;
use engine::analysis::{
    Analyzer, BANDS, FLOOR_DB, MAX_SPECTRA, MAX_WINDOW, band_centre, band_edge, window_len,
};
use engine::device::{AudioIo, BlockCtx, Device, EventBuffer, Transport};
use engine::devices::contour::{self, BandParam, Contour};
use engine::host::{Host, Prepare, channel, read_feedback};
use engine::msg::{ControlMsg as M, DeviceSlot, ParamId, TrackKind, TrackSlot, encode_stream};

const RATE: f32 = 48_000.0;

fn sine(len: usize, hz: f32, sr: f32) -> Vec<f32> {
    (0..len)
        .map(|n| (core::f32::consts::TAU * hz * n as f32 / sr).sin())
        .collect()
}

fn noise(len: usize, seed: u32) -> Vec<f32> {
    let mut s = seed | 1;
    (0..len)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            (s as f32 / u32::MAX as f32) * 2.0 - 1.0
        })
        .collect()
}

/// The band holding `hz`.
fn band_of(hz: f32) -> usize {
    (0..BANDS)
        .find(|&i| band_edge(i) <= hz && hz < band_edge(i + 1))
        .unwrap()
}

#[test]
fn window_lengths_follow_the_sample_rate() {
    assert_eq!(window_len(8_000.0), 2048);
    assert_eq!(window_len(44_100.0), 2048);
    assert_eq!(window_len(48_000.0), 2048);
    assert_eq!(window_len(96_000.0), 4096);
    assert_eq!(window_len(192_000.0), 8192);
    assert_eq!(window_len(f32::NAN), 2048);
    assert_eq!(window_len(1e9), MAX_WINDOW);
}

#[test]
fn the_fft_matches_a_direct_dft() {
    let mut a = Analyzer::try_new().unwrap();
    let x = noise(2048, 5);
    let power: Vec<f32> = a.power(&x).to_vec();
    let n = x.len();
    let mut worst = 0f64;
    for k in [0usize, 1, 7, 100, 511, 1000, 1024] {
        let (mut re, mut im) = (0f64, 0f64);
        for (i, s) in x.iter().enumerate() {
            let w = 0.5 - 0.5 * (core::f64::consts::TAU * i as f64 / n as f64).cos();
            let a = -core::f64::consts::TAU * (k * i) as f64 / n as f64;
            re += f64::from(*s) * w * a.cos();
            im += f64::from(*s) * w * a.sin();
        }
        let scale = 4.0 / n as f64;
        let direct = (re * scale).powi(2) + (im * scale).powi(2);
        let rel = (f64::from(power[k]) - direct).abs() / direct.max(1e-6);
        worst = worst.max(rel);
        assert!(rel < 1e-3, "bin {k}: fft {} direct {direct}", power[k]);
    }
    println!("largest relative gap to the direct DFT: {worst:e}");
}

#[test]
fn a_full_scale_sine_reads_0_dbfs_in_its_band_at_every_rate() {
    let mut a = Analyzer::try_new().unwrap();
    for sr in [44_100.0f32, 48_000.0, 96_000.0, 192_000.0] {
        let n = window_len(sr);
        for hz in [60.0f32, 250.0, 1000.0, 5000.0, 15_000.0] {
            let mut out = [0.0; BANDS];
            a.bands(&sine(n, hz, sr), sr, &mut out);
            let b = band_of(hz);
            // Between bins, a Hann window loses at most 1.42 dB.
            assert!(
                (-1.5..=0.1).contains(&out[b]),
                "{sr} Hz rate, {hz} Hz: {} dB in its band",
                out[b]
            );
            // Far from it, well under (the Hann window's leakage).
            let far = if b > 20 { b - 20 } else { b + 20 };
            assert!(
                out[far] < out[b] - 30.0,
                "{sr}, {hz}: far band {}",
                out[far]
            );
            assert!(out.iter().all(|v| v.is_finite() && *v >= FLOOR_DB));
        }
    }
}

#[test]
fn the_bands_above_nyquist_read_the_floor() {
    let mut a = Analyzer::try_new().unwrap();
    let sr = 22_050.0;
    let mut out = [0.0; BANDS];
    a.bands(&noise(2048, 3), sr, &mut out);
    for (i, v) in out.iter().enumerate() {
        if band_edge(i) >= sr / 2.0 {
            assert_eq!(*v, FLOOR_DB, "band {i}");
        } else {
            assert!(*v > FLOOR_DB, "band {i} has signal");
        }
    }
    // Silence, a NaN-filled window, an empty one: the floor, never a NaN.
    a.bands(&vec![0.0; 2048], RATE, &mut out);
    assert!(out.iter().all(|v| *v == FLOOR_DB));
    a.bands(&vec![f32::NAN; 2048], RATE, &mut out);
    assert!(out.iter().all(|v| *v == FLOOR_DB));
    a.bands(&[], RATE, &mut out);
    assert!(out.iter().all(|v| *v == FLOOR_DB));
}

/// Runs `input` through `c` (mono to both channels) and returns its window.
fn window_after(c: &mut Contour, input: &[f32]) -> Vec<f32> {
    let mut unused = EventBuffer::try_with_capacity(1).unwrap();
    for chunk in input.chunks(128) {
        let (mut l, mut r) = (chunk.to_vec(), chunk.to_vec());
        let ctx = BlockCtx {
            sample_rate: RATE,
            frames: chunk.len() as u32,
            transport: Transport::default(),
        };
        assert_no_alloc(|| {
            c.process(
                &ctx,
                &[],
                &mut AudioIo {
                    left: &mut l,
                    right: &mut r,
                    events_out: &mut unused,
                },
            )
        });
    }
    let mut out = vec![0.0; MAX_WINDOW];
    let n = assert_no_alloc(|| c.analysis_window(&mut out));
    out.truncate(n);
    out
}

#[test]
fn contour_s_window_is_its_last_output_and_a_bell_lifts_its_bands() {
    // The window is the last frames rendered, oldest first.
    let input = noise(5_000, 9);
    let mut flat = Contour::new(RATE);
    let w = window_after(&mut flat, &input);
    assert_eq!(w.len(), window_len(RATE));
    assert_eq!(&w[..], &input[input.len() - w.len()..]);

    // +12 dB at 2 kHz: the bands around it lift by about 12 dB, the bands far below do not.
    let gain = contour::band_param(contour::BELL_2, BandParam::Gain).unwrap();
    let freq = contour::band_param(contour::BELL_2, BandParam::Freq).unwrap();
    let mut a = Analyzer::try_new().unwrap();
    let (mut lift_at, mut lift_low) = (0f32, 0f32);
    for seed in 1..=8 {
        let input = noise(8_000, seed);
        let mut bell = Contour::new(RATE);
        bell.set_param(ParamId(gain), 12.0);
        bell.set_param(ParamId(freq), 2000.0);
        bell.reset();
        let mut flat = Contour::new(RATE);
        let (mut with, mut without) = ([0.0; BANDS], [0.0; BANDS]);
        a.bands(&window_after(&mut bell, &input), RATE, &mut with);
        a.bands(&window_after(&mut flat, &input), RATE, &mut without);
        lift_at += with[band_of(2000.0)] - without[band_of(2000.0)];
        lift_low += with[band_of(150.0)] - without[band_of(150.0)];
    }
    let (lift_at, lift_low) = (lift_at / 8.0, lift_low / 8.0);
    println!("bell +12 dB at 2 kHz: {lift_at:.2} dB at its band, {lift_low:.2} dB at 150 Hz");
    assert!((lift_at - 12.0).abs() < 1.5, "lift {lift_at}");
    assert!(lift_low.abs() < 1.0, "low {lift_low}");
}

#[test]
fn a_reset_hides_the_window_until_the_ring_is_full_again() {
    let mut c = Contour::new(RATE);
    assert!(
        window_after(&mut c, &noise(100, 1)).is_empty(),
        "not full yet"
    );
    assert_eq!(window_after(&mut c, &noise(4_000, 1)).len(), 2048);
    c.reset();
    let mut out = vec![0.0; MAX_WINDOW];
    assert_eq!(c.analysis_window(&mut out), 0);
    assert_eq!(window_after(&mut c, &noise(2_048, 2)).len(), 2048);
    // A shorter buffer takes the last frames that fit.
    let mut short = vec![0.0; 100];
    assert_eq!(c.analysis_window(&mut short), 100);
}

// ---------------------------------------------------------------- through the host

struct Session {
    prepare: Prepare,
    host: Host,
    seq: u32,
}

impl Session {
    fn send(&mut self, msgs: &[M]) {
        let bytes = encode_stream(1, self.seq, msgs).unwrap();
        self.seq += msgs.len() as u32;
        self.prepare.submit(&bytes).unwrap();
    }

    /// Renders `reports` reports' worth of blocks, flushing the prepare side between blocks
    /// as the hosts do, the render under the allocation detector; the last report.
    fn run(&mut self, reports: usize) -> engine::msg::Feedback {
        for _ in 0..reports * 13 {
            assert_no_alloc(|| self.host.render(128));
            self.prepare.flush();
        }
        read_feedback(self.host.feedback()).expect("a report")
    }
}

/// A track with the synth playing a note and `contours` Contours after it, a +12 dB bell at 2
/// kHz on the first.
fn session(contours: u16) -> Session {
    let (prepare, host) = channel(RATE).unwrap();
    let mut s = Session {
        prepare,
        host,
        seq: 0,
    };
    let mut msgs = vec![
        M::Reset { epoch: 1 },
        M::TrackCreate {
            track: TrackSlot(0),
            kind: TrackKind::Midi,
        },
        M::DeviceInsert {
            track: TrackSlot(0),
            device: DeviceSlot(0),
            kind: OSC_SYNTH.kind_id,
            state: Vec::new(),
        },
    ];
    let gain = contour::band_param(contour::BELL_2, BandParam::Gain).unwrap();
    let freq = contour::band_param(contour::BELL_2, BandParam::Freq).unwrap();
    for d in 1..=contours {
        msgs.push(M::DeviceInsert {
            track: TrackSlot(0),
            device: DeviceSlot(d),
            kind: contour::DESCRIPTOR.kind_id,
            state: if d == 1 {
                engine::msg::encode_state(&[(ParamId(gain), 12.0), (ParamId(freq), 2000.0)])
            } else {
                Vec::new()
            },
        });
    }
    msgs.push(M::NoteOn {
        track: TrackSlot(0),
        pitch: 45,
        velocity: 110,
        at: None,
    });
    s.send(&msgs);
    s
}

#[test]
fn the_report_carries_contour_s_spectrum_one_report_after_its_window() {
    let mut s = session(1);
    // The first report: the ring is not full yet (1 600 frames of 2 048), no spectrum.
    let first = s.run(1);
    assert!(first.spectra.is_empty());
    let report = s.run(4);
    assert_eq!(report.spectra.len(), 1, "{:?}", report.spectra.len());
    let sp = &report.spectra[0];
    assert_eq!((sp.track, sp.device), (0, 1));
    assert_eq!(sp.sample_rate, RATE);
    assert!(sp.bands.iter().all(|b| b.is_finite() && *b >= FLOOR_DB));
    // The saw's harmonics are there: something well above the floor in the midrange.
    let mid = sp.bands[band_of(1000.0)];
    assert!(mid > -80.0, "1 kHz band at {mid} dB");
    // Each spectrum is 396 bytes on the wire after the notes: the decode found them whole.
    assert_eq!(engine::analysis::SPECTRUM_WIRE_BYTES, 396);
    // The bands are centred where they say (a sanity check of the scale the page draws).
    assert!((band_centre(0) - 20.0 * 1000f32.powf(0.5 / 96.0)).abs() < 1e-3);
}

#[test]
fn a_bypassed_contour_is_not_analysed_and_comes_back_when_switched_on() {
    let mut s = session(1);
    assert_eq!(s.run(4).spectra.len(), 1);
    s.send(&[M::DeviceSetEnabled {
        track: TrackSlot(0),
        device: DeviceSlot(1),
        enabled: false,
    }]);
    let off = s.run(4);
    assert!(off.spectra.is_empty(), "a bypassed device is not analysed");
    // On again: analysed again once its ring is full (the switch reset it).
    s.send(&[M::DeviceSetEnabled {
        track: TrackSlot(0),
        device: DeviceSlot(1),
        enabled: true,
    }]);
    let again = s.run(6);
    assert_eq!(again.spectra.len(), 1);
    assert_eq!((again.spectra[0].track, again.spectra[0].device), (0, 1));
}

#[test]
fn with_more_contours_than_a_report_carries_each_gets_its_turn_also_when_some_leave() {
    let mut s = session(6);
    let mut seen = std::collections::BTreeSet::new();
    for _ in 0..6 {
        let r = s.run(1);
        assert!(r.spectra.len() <= MAX_SPECTRA);
        seen.extend(r.spectra.iter().map(|sp| sp.device));
    }
    assert_eq!(seen, (1..=6).collect(), "every Contour was analysed");
    // Down to two while the round robin points past them: both still come back.
    for d in 3..=6 {
        s.send(&[M::DeviceRemove {
            track: TrackSlot(0),
            device: DeviceSlot(d),
        }]);
    }
    // The report that follows still carries the spectra computed before the change (one report
    // late, as every spectrum): the page drops those of a slot it no longer maps.
    s.run(1);
    let mut seen = std::collections::BTreeSet::new();
    for _ in 0..3 {
        let r = s.run(1);
        seen.extend(r.spectra.iter().map(|sp| sp.device));
    }
    assert_eq!(seen, [1, 2].into());
}

#[test]
fn without_a_contour_the_report_carries_no_spectrum_and_the_analysis_does_nothing() {
    let (mut prepare, mut host) = channel(RATE).unwrap();
    let bytes = encode_stream(
        1,
        0,
        &[
            M::Reset { epoch: 1 },
            M::TrackCreate {
                track: TrackSlot(0),
                kind: TrackKind::Midi,
            },
        ],
    )
    .unwrap();
    prepare.submit(&bytes).unwrap();
    for _ in 0..30 {
        host.render(128);
        prepare.flush();
    }
    let report = read_feedback(host.feedback()).unwrap();
    assert!(report.spectra.is_empty());
    // Nothing new to analyse: a second call publishes nothing.
    prepare.analyse();
    assert!(!prepare.analyse());
}
