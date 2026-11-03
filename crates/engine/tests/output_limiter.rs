//! The output protection (#402) through the shipping host: nothing leaves the engine above
//! the ceiling, however hot the mix, while the output's meter keeps the level before it; a
//! mix under the ceiling passes untouched; the audio path does not allocate with it; the
//! protection adds no latency.

#[cfg(debug_assertions)]
use assert_no_alloc::AllocDisabler;
use assert_no_alloc::assert_no_alloc;
use engine::host::{Host, Prepare, channel, read_feedback};
use engine::limiter::{CEILING, CEILING_DB, Limiter};
use engine::msg::{
    ControlMsg as M, DeviceSlot, ParamId, TrackKind, TrackSlot, encode_stream, limits,
};
use engine::time::SampleRate;
use engine::{OSC_SYNTH, Param};

#[cfg(debug_assertions)]
#[global_allocator]
static A: AllocDisabler = AllocDisabler;

const RATE: f32 = 48_000.0;

struct Rig {
    prepare: Prepare,
    host: Host,
    seq: u32,
    /// The highest output peak the reports showed so far.
    metered: f32,
}

impl Rig {
    /// `tracks` synth tracks, each playing a four-note chord, under a master of `master`
    /// (linear gain).
    fn new(tracks: u32, master: f32) -> Rig {
        let (prepare, host) = channel(RATE).unwrap();
        let mut rig = Rig {
            prepare,
            host,
            seq: 0,
            metered: 0.0,
        };
        rig.send(&[
            M::Reset { epoch: 1 },
            M::SetMaster {
                gain: master,
                pan: 0.0,
                mute: false,
            },
        ]);
        for t in 0..tracks {
            let track = TrackSlot(t);
            rig.send(&[
                M::TrackCreate {
                    track,
                    kind: TrackKind::Midi,
                },
                M::DeviceInsert {
                    track,
                    device: DeviceSlot(0),
                    kind: OSC_SYNTH.kind_id,
                    state: Vec::new(),
                },
                M::ParamSet {
                    track,
                    device: DeviceSlot(0),
                    param: ParamId(Param::Gain.id()),
                    value: 1.0,
                },
            ]);
            for pitch in [48, 55, 60, 64] {
                rig.send(&[M::NoteOn {
                    track,
                    pitch,
                    velocity: 127,
                    at: None,
                }]);
            }
        }
        rig
    }

    fn send(&mut self, msgs: &[M]) {
        let bytes = encode_stream(1, self.seq, msgs).unwrap();
        self.seq += msgs.len() as u32;
        self.prepare.submit(&bytes).unwrap();
    }

    /// Renders `seconds` in blocks of 1 to 512 frames (the allocation detector on in
    /// debug); returns both channels.
    fn render(&mut self, seconds: f32) -> (Vec<f32>, Vec<f32>) {
        let total = (seconds * RATE) as usize;
        let (mut l, mut r) = (
            Vec::with_capacity(total + 600),
            Vec::with_capacity(total + 600),
        );
        let mut block = 0usize;
        while l.len() < total {
            self.prepare.flush();
            let frames = 1 + (block * 37) % 512;
            block += 1;
            let n = assert_no_alloc(|| self.host.render(frames));
            l.extend_from_slice(&self.host.left()[..n]);
            r.extend_from_slice(&self.host.right()[..n]);
            self.watch_meter();
        }
        (l, r)
    }

    /// Reads the latest report (the host writes one every few thousand frames and starts a
    /// new meter period each time) and keeps the highest output peak.
    fn watch_meter(&mut self) {
        if let Some(fb) = read_feedback(self.host.feedback()) {
            self.metered = self.metered.max(fb.master.peak_l).max(fb.master.peak_r);
        }
    }

    /// The highest output peak of all the reports, the last period included.
    fn meter_peak(&mut self) -> f32 {
        self.host.write_feedback();
        self.watch_meter();
        self.metered
    }
}

fn peak(l: &[f32], r: &[f32]) -> f32 {
    l.iter().chain(r).fold(0.0f32, |p, s| p.max(s.abs()))
}

#[test]
fn a_hot_mix_never_leaves_above_the_ceiling_and_the_meter_shows_the_overload() {
    // Five chords of full-level synths under a master 20 dB up.
    let mut rig = Rig::new(5, 10.0);
    let (l, r) = rig.render(3.0);
    let out = peak(&l, &r);
    assert!(
        out <= CEILING,
        "output peak {out} above the ceiling {CEILING}"
    );
    assert!(
        out > 0.9,
        "the limiter works at the ceiling, not far under it: {out}"
    );
    assert!(l.iter().chain(&r).all(|s| s.is_finite()));
    // The meter is before the protection: it reads the overload (and the clip mark lights
    // at 0.999).
    let metered = rig.meter_peak();
    assert!(
        metered > 1.5,
        "the meter shows {metered}, the level before the protection"
    );
    assert_eq!(rig.host.engine().skipped(), 0);
    // The synths are finite, however hot: the limiter clamps, it silences nothing.
    assert_eq!(rig.host.engine().limiter_silenced(), 0);
}

#[test]
fn a_mix_under_the_ceiling_passes_untouched_and_the_meter_agrees() {
    let mut rig = Rig::new(1, 0.5);
    let (l, r) = rig.render(2.0);
    let out = peak(&l, &r);
    assert!(
        out > 0.05 && out < CEILING,
        "the mix is audible and under the ceiling: {out}"
    );
    // The meter is before the protection; with the protection at rest the two are the
    // same signal, so the peaks are the same number.
    let metered = rig.meter_peak();
    assert!((metered - out).abs() < 1e-6, "{metered} vs {out}");
}

#[test]
fn the_protection_adds_no_latency() {
    // (a) Stage level: the attack has no look-ahead, and a quiet sample is not delayed.
    let mut lim = Limiter::new(SampleRate::HZ_48000);
    assert_eq!(Limiter::latency_frames(), 0);
    let mut l = vec![0.0f32; 64];
    let mut r = vec![0.0f32; 64];
    l[17] = 3.0;
    l[40] = 0.25;
    lim.process(&mut l, &mut r);
    assert!(l[17] > 0.9 && l[17] <= CEILING, "{}", l[17]);
    assert!(l[16] == 0.0 && l[18] == 0.0);
    // The gain is still down 23 samples later (the release is tens of milliseconds): the
    // quiet sample is at its own position, scaled, not moved.
    assert!(l[40] > 0.05 && l[40] < 0.25, "{}", l[40]);

    // (b) Engine level: the onset frame is the same in two independent rigs (one quiet,
    // one hot), which have the same limiter state (both at rest). This shows the engine's
    // render topology does not buffer samples around the limiter stage; it does not compare
    // "with limiter" against "without limiter" (the engine always has one). The stage-level
    // evidence (a) is what proves zero latency: the first over is on its own sample, nothing
    // is shifted in the output.
    let onset_frame = |l: &[f32], r: &[f32]| {
        l.iter()
            .zip(r)
            .position(|(a, b)| a.abs() > 1e-9 || b.abs() > 1e-9)
    };
    // Under the ceiling: onset at some frame F.
    let (l_under, r_under) = {
        let mut rig = Rig::new(1, 0.3);
        rig.render(0.05)
    };
    // Far over the ceiling: onset at the same frame F (both rigs start with limiter at rest).
    let (l_over, r_over) = {
        let mut rig = Rig::new(1, 5.0);
        rig.render(0.05)
    };
    assert!(peak(&l_over, &r_over) <= CEILING);
    assert!(peak(&l_under, &r_under) < CEILING);
    let f_under = onset_frame(&l_under, &r_under);
    let f_over = onset_frame(&l_over, &r_over);
    assert_eq!(
        f_under, f_over,
        "onset at frame {f_under:?} (quiet) vs {f_over:?} (hot): engine topology shifted audio"
    );
}

#[test]
fn the_ceiling_is_minus_0_3_dbfs() {
    let db = 20.0 * f64::from(CEILING).log10();
    assert!((db - f64::from(CEILING_DB)).abs() < 1e-5, "{db}");
}

#[test]
fn many_tracks_hot_and_growth_never_allocate() {
    let mut rig = Rig::new(4, 8.0);
    for t in 4..40 {
        let track = TrackSlot(t);
        rig.send(&[
            M::TrackCreate {
                track,
                kind: TrackKind::Midi,
            },
            M::DeviceInsert {
                track,
                device: DeviceSlot(0),
                kind: OSC_SYNTH.kind_id,
                state: Vec::new(),
            },
            M::NoteOn {
                track,
                pitch: 40 + t as u8,
                velocity: 127,
                at: None,
            },
        ]);
    }
    const { assert!(limits::MAX_TRACKS >= 40) };
    let (l, r) = rig.render(1.0);
    assert!(peak(&l, &r) <= CEILING);
}

// ---------------------------------------------------------------- engine-level policy tests

#[test]
fn after_all_sound_off_the_limiter_is_at_rest() {
    // A hot mix engages the limiter; AllSoundOff fades the output to silence and resets every
    // device, including the limiter; the gain returns to exactly 1.0.
    let mut rig = Rig::new(3, 8.0);
    rig.render(0.3);
    // The limiter must be limiting.
    assert!(
        rig.host.engine().limiter_gain() < 1.0,
        "pre-cut gain not limited"
    );
    // AllSoundOff: fades over graph::FADE_SECONDS (5 ms), then resets.
    rig.send(&[M::AllSoundOff]);
    // Render well past the fade (5 ms ≈ 240 frames at 48 kHz; 512 is more than enough).
    rig.render(0.1);
    assert_eq!(
        rig.host.engine().limiter_gain(),
        1.0,
        "limiter not at rest after AllSoundOff"
    );
}

#[test]
fn on_project_reset_the_limiter_keeps_its_gain() {
    // A hot project replaces a quiet one: the retiring tracks fade through the same limiter,
    // so there must be no gain jump. The limiter's deficit survives a Reset.
    let mut rig = Rig::new(3, 8.0);
    rig.render(0.3);
    let gain_before = rig.host.engine().limiter_gain();
    assert!(gain_before < 1.0, "pre-reset gain not limited");
    // Reset to a new epoch (simulates opening a new project).
    rig.send(&[M::Reset { epoch: 2 }]);
    rig.prepare.flush();
    // One pump to apply the reset.
    rig.host.pump();
    let gain_after = rig.host.engine().limiter_gain();
    assert_eq!(
        gain_after, gain_before,
        "limiter gain changed on Reset: {} → {}",
        gain_before, gain_after
    );
}

#[test]
fn set_sample_rate_is_forwarded_to_the_limiter() {
    // Engine::set_sample_rate must forward the rate to its limiter, so the release keeps its
    // 80 ms duration whatever the rate. This is pinned by measuring the release of the engine's
    // *own* limiter (not a standalone one): a hot mix engages it, the master is dropped to
    // silence so the output falls below the ceiling and the limiter releases, and the number of
    // samples the engine's limiter_gain() takes to decay by 1/e gives the time constant. Built
    // at 48 kHz then switched to 96 kHz: if the forwarding call (engine.rs set_sample_rate) was
    // removed, the engine's limiter would keep its 48 kHz coefficient and the measured constant
    // would be about twice RELEASE_SECONDS, failing the assertion.
    use engine::limiter::RELEASE_SECONDS;
    const NEW_RATE: u32 = 96_000;

    let mut rig = Rig::new(3, 8.0);
    // Switch to 96 kHz via Engine::set_sample_rate, then engage the limiter at the new rate.
    rig.host
        .engine_mut()
        .set_sample_rate(SampleRate::new(NEW_RATE).unwrap());
    rig.render(0.1);
    let engaged = rig.host.engine().limiter_gain();
    assert!(engaged < 1.0, "limiter not engaged after rate change");

    // Cut the output to silence so the engine's limiter releases: mute the master. The detector
    // then sees 0, the deficit shrinks by `keep` every sample, and limiter_gain() climbs to 1.0.
    rig.send(&[M::SetMaster {
        gain: 0.0,
        pan: 0.0,
        mute: true,
    }]);
    rig.prepare.flush();
    rig.host.pump();
    // Let the master fade settle to silence before timing the release (the fade itself is a
    // few milliseconds; the deficit measured at the start of the timing window is `d0`).
    rig.render(0.02);
    let d0 = 1.0 - rig.host.engine().limiter_gain();
    assert!(
        d0 > 0.0,
        "limiter already at rest before the release window"
    );

    // Advance block by block until the engine's limiter deficit has decayed by 1/e.
    let mut frames = 0u32;
    loop {
        rig.prepare.flush();
        let n = rig.host.render(64);
        frames += n as u32;
        if 1.0 - rig.host.engine().limiter_gain() <= d0 / std::f32::consts::E {
            break;
        }
        assert!(
            frames < NEW_RATE,
            "release never reached 1/e within one second"
        );
    }
    let t = f64::from(frames) / f64::from(NEW_RATE);
    // One 64-frame block of granularity on a ~80 ms constant is about 0.7 ms; allow 3 ms.
    assert!(
        (t - RELEASE_SECONDS).abs() < 0.003,
        "engine limiter release at {NEW_RATE} Hz: {t:.4} s (expected {RELEASE_SECONDS}); \
         forwarding to the limiter is likely broken (engaged gain was {engaged})"
    );
}

/// The cost of the stage per block of 128 frames at 48 kHz, at rest and while limiting, and
/// of a whole engine block for scale. Run: `cargo test -p engine --release --test
/// output_limiter -- --ignored --nocapture`. Numbers depend on the machine.
#[test]
#[ignore = "measures time"]
fn the_cost_of_the_stage_per_block() {
    use std::hint::black_box;
    use std::time::Instant;
    let blocks = 200_000u32;
    let time = |hot: bool| {
        let mut lim = Limiter::new(SampleRate::HZ_48000);
        let amp = if hot { 4.0f32 } else { 0.5 };
        let mut l = [0.0f32; 128];
        let mut r = [0.0f32; 128];
        let start = Instant::now();
        for _ in 0..blocks {
            for (i, (a, b)) in l.iter_mut().zip(r.iter_mut()).enumerate() {
                *a = amp * (i as f32 * 0.3).sin();
                *b = amp * (i as f32 * 0.2).cos();
            }
            lim.process(black_box(&mut l), black_box(&mut r));
            black_box((&l, &r));
        }
        start.elapsed().as_nanos() as f64 / f64::from(blocks)
    };
    // The generator loop alone, to subtract.
    let base = {
        let mut l = [0.0f32; 128];
        let mut r = [0.0f32; 128];
        let start = Instant::now();
        for _ in 0..blocks {
            for (i, (a, b)) in l.iter_mut().zip(r.iter_mut()).enumerate() {
                *a = 0.5 * (i as f32 * 0.3).sin();
                *b = 0.5 * (i as f32 * 0.2).cos();
            }
            black_box((&l, &r));
        }
        start.elapsed().as_nanos() as f64 / f64::from(blocks)
    };
    let (rest, hot) = (time(false) - base, time(true) - base);
    let mut rig = Rig::new(5, 1.0);
    rig.render(0.5);
    let start = Instant::now();
    let n = 20_000;
    for _ in 0..n {
        rig.host.render(128);
    }
    let engine_block = start.elapsed().as_nanos() as f64 / f64::from(n);
    let budget = 128.0 / f64::from(RATE) * 1e9;
    println!(
        "limiter per 128-frame block: at rest {rest:.0} ns, limiting {hot:.0} ns \
         ({:.3} % and {:.3} % of the {budget:.0} ns budget); whole engine block with 5 chord tracks {engine_block:.0} ns",
        rest / budget * 100.0,
        hot / budget * 100.0
    );
}
