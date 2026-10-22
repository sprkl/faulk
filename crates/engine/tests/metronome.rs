//! The metronome (TR-2) and the wiring of the clock (TR-1) in the engine: offline renders
//! through the real `Preparer`, `Engine::apply` and `Engine::render`, every audio-side
//! call under the allocation detector. Asserts on the onset sample of each click (the
//! exact sample), the accent, 4/4 and 6/8, loops, tempo and sample-rate changes, silence
//! when disabled, the output after the master, the count-in and the report of the
//! playhead.

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use engine::device::DeviceSetup;
use engine::device::testing::{TONE, factory};
use engine::engine::{Engine, Preparer, channel};
use engine::metronome::{ACCENT_PEAK, BEAT_PEAK, CLICK_SECONDS};
use engine::msg::{
    ControlMsg as M, DeviceSlot, Feedback, TrackKind, TrackSlot, TransportState, encode_state,
    limits,
};
use engine::time::{LoopRegion, Quantize, SampleRate, Tempo, Tick, TimeSignature};

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

const SIZES: [usize; 7] = [1, 7, 127, 480, 64, 2000, 333];

struct Rig {
    p: Preparer,
    e: Engine,
    rate: f64,
    /// Where `render` takes its block sizes.
    next_size: usize,
}

impl Rig {
    fn new(rate: u32) -> Rig {
        let setup = DeviceSetup {
            sample_rate: rate as f32,
            max_block_frames: limits::MAX_BLOCK_FRAMES,
        };
        let (p, e) = channel(setup, factory).unwrap();
        Rig {
            p,
            e,
            rate: f64::from(rate),
            next_size: 0,
        }
    }

    fn send(&mut self, msgs: &[M]) {
        for m in msgs {
            self.p.handle(m.clone()).unwrap();
        }
        self.p.flush();
    }

    /// `frames` frames in blocks of the sizes of [`SIZES`]; returns the left channel
    /// (checked equal to the right one: the click is centred).
    fn render(&mut self, frames: usize) -> Vec<f32> {
        let mut out = Vec::with_capacity(frames);
        while out.len() < frames {
            let n = SIZES[self.next_size % SIZES.len()].min(frames - out.len());
            self.next_size += 1;
            let (mut l, mut r) = (vec![0.0f32; n], vec![0.0f32; n]);
            assert_no_alloc(|| self.e.process(&mut l, &mut r));
            assert_eq!(l, r);
            out.extend_from_slice(&l);
            self.p.collect();
        }
        assert!(
            out.iter()
                .all(|s| s.is_finite() && (*s == 0.0 || s.abs() > 1e-20))
        );
        out
    }

    fn seconds(&mut self, s: f64) -> Vec<f32> {
        self.render((s * self.rate).round() as usize)
    }

    fn setup(&mut self, bpm: f64, sig: (u8, u8), volume: f32) {
        self.send(&[
            M::Reset { epoch: 1 },
            M::SetTransport {
                tempo: Tempo::new(bpm).unwrap(),
                signature: TimeSignature::new(sig.0, sig.1).unwrap(),
                launch_quantize: Quantize::default(),
            },
            M::SetMetronome {
                enabled: true,
                volume,
                count_in_bars: 0,
            },
        ]);
    }
}

/// The onsets of a render: (sample, first sample's value).
fn onsets(out: &[f32]) -> Vec<(usize, f32)> {
    let gap = (CLICK_SECONDS * 192_000.0) as usize;
    let mut found = Vec::new();
    let mut quiet = gap;
    for (i, s) in out.iter().enumerate() {
        if *s != 0.0 {
            if quiet >= gap {
                found.push((i, *s));
            }
            quiet = 0;
        } else {
            quiet = quiet.saturating_add(1);
        }
    }
    found
}

fn is_accent(v: f32) -> bool {
    (f64::from(v) - ACCENT_PEAK).abs() < 1e-6
}

fn is_beat(v: f32) -> bool {
    (f64::from(v) - BEAT_PEAK).abs() < 1e-6
}

/// Samples per beat of `ticks` ticks at `bpm`.
fn beat_samples(rate: f64, bpm: f64, ticks: f64) -> f64 {
    ticks * 60.0 * rate / (bpm * 960.0)
}

#[test]
fn clicks_land_on_the_exact_sample_of_each_beat_in_4_4() {
    for rate in [44_100u32, 48_000, 96_000] {
        for bpm in [100.0, 120.0, 97.3, 133.7] {
            let mut rig = Rig::new(rate);
            rig.setup(bpm, (4, 4), 1.0);
            rig.send(&[M::Play { from: Tick(0) }]);
            let out = rig.seconds(10.0);
            let found = onsets(&out);
            let beats = (10.0 * bpm / 60.0).floor() as usize;
            assert!(found.len() >= beats, "{rate} {bpm}: {} clicks", found.len());
            for (k, (at, v)) in found.iter().enumerate() {
                let expected = (k as f64 * beat_samples(f64::from(rate), bpm, 960.0)).floor();
                assert_eq!(*at as f64, expected, "{rate} Hz {bpm} BPM beat {k}");
                // The accent is the first beat of the bar and only that one.
                assert_eq!(is_accent(*v), k % 4 == 0, "{rate} {bpm} beat {k}: {v}");
                assert_eq!(is_beat(*v), k % 4 != 0, "{rate} {bpm} beat {k}: {v}");
            }
        }
    }
}

#[test]
fn six_eight_has_six_eighth_note_beats_and_one_accent() {
    for rate in [44_100u32, 48_000] {
        let bpm = 90.0;
        let mut rig = Rig::new(rate);
        rig.setup(bpm, (6, 8), 1.0);
        rig.send(&[M::Play { from: Tick(0) }]);
        let out = rig.seconds(12.0);
        let found = onsets(&out);
        assert!(found.len() > 24);
        for (k, (at, v)) in found.iter().enumerate() {
            let expected = (k as f64 * beat_samples(f64::from(rate), bpm, 480.0)).floor();
            assert_eq!(*at as f64, expected, "{rate} beat {k}");
            assert_eq!(is_accent(*v), k % 6 == 0, "beat {k}");
        }
    }
}

#[test]
fn a_play_from_the_middle_of_a_bar_accents_the_next_bar_line() {
    let mut rig = Rig::new(48_000);
    rig.setup(120.0, (4, 4), 1.0);
    // Tick 2 * 960 is the third beat; the next click is on it, the accent is 2 beats later.
    rig.send(&[M::Play { from: Tick(1920) }]);
    let out = rig.seconds(3.0);
    let found = onsets(&out);
    assert_eq!(found[0].0, 0);
    assert!(is_beat(found[0].1));
    assert_eq!(found[1].0, 24_000);
    assert!(is_beat(found[1].1));
    assert_eq!(found[2].0, 48_000);
    assert!(is_accent(found[2].1));
}

#[test]
fn clicks_follow_a_loop_without_gap_or_duplicate() {
    // 4 beats at 120 BPM and 48 kHz: 96 000 samples, exact, so every click is exact.
    let mut rig = Rig::new(48_000);
    rig.setup(120.0, (4, 4), 1.0);
    rig.send(&[
        M::SetLoop {
            region: LoopRegion::new(Tick(1920), Tick(5760)),
        },
        M::Play { from: Tick(0) },
    ]);
    let out = rig.seconds(30.0);
    let found = onsets(&out);
    assert_eq!(found.len(), 60);
    for (k, (at, v)) in found.iter().enumerate() {
        assert_eq!(*at, k * 24_000, "click {k}");
        // The loop start (tick 1920, a beat) is no bar line; the bar line (tick 3840)
        // recurs every 4 clicks from the 5th: click 0 is the bar line of the first pass.
        assert_eq!(is_accent(*v), k % 4 == 0, "click {k}");
    }
}

#[test]
fn clicks_follow_a_loop_at_a_fractional_tempo_within_one_sample() {
    let (rate, bpm) = (44_100u32, 97.3);
    let mut rig = Rig::new(rate);
    rig.setup(bpm, (4, 4), 1.0);
    rig.send(&[
        M::SetLoop {
            region: LoopRegion::new(Tick(0), Tick(3840)),
        },
        M::Play { from: Tick(0) },
    ]);
    let out = rig.seconds(40.0);
    let found = onsets(&out);
    let beat = beat_samples(f64::from(rate), bpm, 960.0);
    assert!(found.len() > 60);
    for (k, (at, v)) in found.iter().enumerate() {
        let exact = k as f64 * beat;
        // The loop is exactly its length in samples; the click after a wrap is clamped
        // into the first sample of its segment (at most one sample late).
        assert!(
            (*at as f64 - exact.floor()).abs() <= 1.0,
            "click {k}: {at} vs {exact}"
        );
        assert_eq!(is_accent(*v), k % 4 == 0, "click {k}");
    }
}

#[test]
fn a_tempo_change_moves_the_next_clicks_without_a_gap() {
    let mut rig = Rig::new(48_000);
    rig.setup(120.0, (4, 4), 1.0);
    rig.send(&[M::Play { from: Tick(0) }]);
    let mut out = rig.seconds(1.0);
    rig.send(&[M::SetTransport {
        tempo: Tempo::new(60.0).unwrap(),
        signature: TimeSignature::COMMON,
        launch_quantize: Quantize::default(),
    }]);
    out.extend(rig.seconds(4.0));
    let found: Vec<usize> = onsets(&out).iter().map(|o| o.0).collect();
    // 120 BPM: 0 and 24 000; the change lands at the next block (the 1-second render
    // ends at 48 000); the beat at 48 000 is the first of the new tempo, then every
    // 48 000 samples.
    assert_eq!(found[..2], [0, 24_000]);
    assert_eq!(found[2], 48_000);
    assert_eq!(found[3], 96_000);
    assert_eq!(found[4], 144_000);
}

#[test]
fn a_disabled_metronome_is_bit_exact_silence() {
    let mut rig = Rig::new(48_000);
    rig.setup(120.0, (4, 4), 1.0);
    rig.send(&[
        M::SetMetronome {
            enabled: false,
            volume: 1.0,
            count_in_bars: 2,
        },
        M::Play { from: Tick(0) },
    ]);
    let out = rig.seconds(5.0);
    assert!(out.iter().all(|s| s.to_bits() == 0));
}

#[test]
fn volume_zero_or_unusable_is_silence_and_volume_scales_the_click() {
    for volume in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        let mut rig = Rig::new(48_000);
        rig.setup(120.0, (4, 4), volume);
        rig.send(&[M::Play { from: Tick(0) }]);
        let out = rig.seconds(2.0);
        assert!(out.iter().all(|s| *s == 0.0), "volume {volume}");
    }
    let mut rig = Rig::new(48_000);
    rig.setup(120.0, (4, 4), 0.5);
    rig.send(&[M::Play { from: Tick(0) }]);
    let found = onsets(&rig.seconds(1.5));
    assert!((f64::from(found[0].1) - ACCENT_PEAK * 0.5).abs() < 1e-6);
    assert!((f64::from(found[1].1) - BEAT_PEAK * 0.5).abs() < 1e-6);
}

#[test]
fn a_stopped_clock_is_silent_and_disabling_cuts_a_click_in_progress() {
    let mut rig = Rig::new(48_000);
    rig.setup(120.0, (4, 4), 1.0);
    assert!(rig.seconds(1.0).iter().all(|s| *s == 0.0));
    rig.send(&[M::Play { from: Tick(0) }]);
    let started = rig.render(100);
    assert!(started[99] != 0.0, "a click is in progress");
    rig.send(&[M::SetMetronome {
        enabled: false,
        volume: 1.0,
        count_in_bars: 0,
    }]);
    assert!(rig.seconds(3.0).iter().all(|s| *s == 0.0));
    // Stop: a click in progress rings out, then nothing.
    rig.send(&[
        M::SetMetronome {
            enabled: true,
            volume: 1.0,
            count_in_bars: 0,
        },
        M::Play { from: Tick(0) },
    ]);
    rig.render(100);
    rig.send(&[M::Stop]);
    let after = rig.render(2000);
    assert!(after[..500].iter().any(|s| *s != 0.0));
    assert!(after[1500..].iter().all(|s| *s == 0.0));
    assert!(rig.seconds(3.0).iter().all(|s| *s == 0.0));
}

#[test]
fn the_metronome_is_after_the_master_and_beside_the_tracks() {
    fn with_track(metronome: bool, master_mute: bool) -> Vec<f32> {
        let mut rig = Rig::new(48_000);
        rig.setup(120.0, (4, 4), 1.0);
        rig.send(&[
            M::SetMetronome {
                enabled: metronome,
                volume: 1.0,
                count_in_bars: 0,
            },
            M::SetMaster {
                gain: 0.5,
                pan: 0.0,
                mute: master_mute,
            },
            M::TrackCreate {
                track: TrackSlot(0),
                kind: TrackKind::Midi,
            },
            M::DeviceInsert {
                track: TrackSlot(0),
                device: DeviceSlot(0),
                kind: TONE.kind_id,
                state: encode_state(&[]),
            },
            M::NoteOn {
                track: TrackSlot(0),
                pitch: 57,
                velocity: 100,
            },
            M::Play { from: Tick(0) },
        ]);
        rig.seconds(2.0)
    }
    let alone = {
        let mut rig = Rig::new(48_000);
        rig.setup(120.0, (4, 4), 1.0);
        rig.send(&[M::Play { from: Tick(0) }]);
        rig.seconds(2.0)
    };
    // The metronome at master gain 0.5 and with the master muted is the same click: the
    // master touches the tracks only.
    let (without, with, muted) = (
        with_track(false, false),
        with_track(true, false),
        with_track(true, true),
    );
    assert!(without.iter().any(|s| s.abs() > 0.01), "the track sounds");
    // (after the fade of the master: the mute is a ramp)
    assert!(
        muted[4800..]
            .iter()
            .zip(&alone[4800..])
            .all(|(m, a)| (m - a).abs() < 1e-6)
    );
    for ((w, wo), a) in with.iter().zip(&without).zip(&alone) {
        assert!((w - wo - a).abs() < 1e-6, "the sum is track plus click");
    }
}

#[test]
fn a_count_in_runs_the_playhead_through_negative_ticks_before_a_recording() {
    let mut rig = Rig::new(48_000);
    rig.setup(120.0, (4, 4), 1.0);
    rig.send(&[
        M::SetMetronome {
            enabled: true,
            volume: 1.0,
            count_in_bars: 2,
        },
        M::SetRecording { recording: true },
        M::Play { from: Tick(0) },
    ]);
    let first = rig.render(64);
    assert!(is_accent(first[0]), "the count-in starts on a bar line");
    assert_eq!(rig.e.clock().playhead_tick(), Tick(-7680 + 2));
    let mut out = first;
    out.extend(rig.seconds(6.0));
    let found = onsets(&out);
    // Two bars of count-in: 8 clicks, then the recording starts at the cue, on a click.
    for (k, (at, v)) in found.iter().take(10).enumerate() {
        assert_eq!(*at, k * 24_000, "click {k}");
        assert_eq!(is_accent(*v), k % 4 == 0);
    }
    // The cue (tick 0) is played at sample 8 * 24 000.
    let mut r = Feedback::default();
    rig.e.report_transport(&mut r);
    assert_eq!(r.transport, TransportState::Recording);
    assert!(r.playhead_tick.0 > 0, "past the cue");
}

#[test]
fn the_playhead_reaches_the_cue_when_the_count_in_ends() {
    let mut rig = Rig::new(48_000);
    rig.setup(120.0, (4, 4), 1.0);
    rig.send(&[
        M::SetMetronome {
            enabled: true,
            volume: 1.0,
            count_in_bars: 1,
        },
        M::SetRecording { recording: true },
        M::Play { from: Tick(3840) },
    ]);
    rig.render(96_000 - 1);
    assert_eq!(rig.e.clock().playhead_tick(), Tick(3840 - 1));
    rig.render(1);
    assert_eq!(rig.e.clock().playhead_tick(), Tick(3840));
}

#[test]
fn no_count_in_when_disabled_not_recording_or_already_playing() {
    // Disabled: silent and the playhead starts at the cue (64 frames are 2 ticks).
    let mut rig = Rig::new(48_000);
    rig.setup(120.0, (4, 4), 1.0);
    rig.send(&[
        M::SetMetronome {
            enabled: false,
            volume: 1.0,
            count_in_bars: 2,
        },
        M::SetRecording { recording: true },
        M::Play { from: Tick(0) },
    ]);
    rig.render(64);
    assert_eq!(rig.e.clock().playhead_tick(), Tick(2));
    // Not recording: no pre-roll.
    let mut rig = Rig::new(48_000);
    rig.setup(120.0, (4, 4), 1.0);
    rig.send(&[
        M::SetMetronome {
            enabled: true,
            volume: 1.0,
            count_in_bars: 2,
        },
        M::Play { from: Tick(0) },
    ]);
    rig.render(64);
    assert_eq!(rig.e.clock().playhead_tick(), Tick(2));
    // Already playing: a new Play (or a Seek) is not preceded by a count-in.
    rig.send(&[
        M::SetRecording { recording: true },
        M::Play { from: Tick(1920) },
    ]);
    rig.render(64);
    assert_eq!(rig.e.clock().playhead_tick(), Tick(1922));
    // A count-in of zero bars: none.
    let mut rig = Rig::new(48_000);
    rig.setup(120.0, (4, 4), 1.0);
    rig.send(&[
        M::SetRecording { recording: true },
        M::Play { from: Tick(0) },
    ]);
    rig.render(64);
    assert_eq!(rig.e.clock().playhead_tick(), Tick(2));
}

#[test]
fn the_report_carries_the_state_and_the_playhead() {
    let mut rig = Rig::new(48_000);
    rig.setup(120.0, (4, 4), 1.0);
    let mut r = Feedback::default();
    rig.e.report_transport(&mut r);
    assert_eq!(r.transport, TransportState::Stopped);
    assert_eq!((r.playhead_tick, r.playhead_frame.get()), (Tick(0), 0));
    rig.send(&[M::Play { from: Tick(960) }]);
    rig.render(48_000);
    assert_no_alloc(|| rig.e.report_transport(&mut r));
    assert_eq!(r.transport, TransportState::Playing);
    // 1 s at 120 BPM is 2 beats: from tick 960 to tick 2880, frame 2880 * 25.
    assert_eq!(r.playhead_tick, Tick(2880));
    assert_eq!(r.playhead_frame.get(), 2880 * 25);
    rig.send(&[M::Stop]);
    rig.render(10);
    rig.e.report_transport(&mut r);
    assert_eq!(r.transport, TransportState::Stopped);
    assert_eq!(r.playhead_tick, Tick(2880));
    rig.send(&[
        M::SetRecording { recording: true },
        M::Play { from: Tick(0) },
    ]);
    rig.render(10);
    rig.e.report_transport(&mut r);
    assert_eq!(r.transport, TransportState::Recording);
}

#[test]
fn a_sample_rate_change_keeps_the_playhead_and_the_beat() {
    for (from, to) in [(48_000u32, 44_100u32), (44_100, 96_000), (96_000, 48_000)] {
        let bpm = 120.0;
        let mut rig = Rig::new(from);
        rig.setup(bpm, (4, 4), 1.0);
        rig.send(&[M::Play { from: Tick(0) }]);
        // 1.3 s: the position is a third of a beat past the beat at 1.0 s.
        let before = rig.seconds(1.3);
        let pos = rig.e.clock().position();
        rig.e.set_sample_rate(SampleRate::new(to).unwrap());
        assert_eq!(rig.e.clock().position(), pos, "the playhead did not move");
        assert_eq!(rig.e.clock().sample_rate(), SampleRate::new(to).unwrap());
        rig.rate = f64::from(to);
        let after = rig.seconds(2.0);
        let found = onsets(&after);
        // Ticks to the next beat, then one beat per `to / 2` samples.
        let spt = 60.0 * f64::from(to) / (bpm * 960.0);
        let next_beat = (pos / 960.0).ceil() * 960.0;
        let first = ((next_beat - pos) * spt).floor() as usize;
        // A click in progress at the change is dropped; the one before it is not
        // counted here.
        let beat = f64::from(to) / 2.0;
        assert!(
            found[0].0.abs_diff(first) <= 1,
            "{from}->{to}: first click {} vs {first}",
            found[0].0
        );
        for (k, (at, _)) in found.iter().enumerate() {
            let expected = first as f64 + k as f64 * beat;
            assert!(
                (*at as f64 - expected).abs() <= 1.0,
                "{from}->{to} click {k}"
            );
        }
        // The beats before the change were all heard: 0 and 1 beat at 120 BPM.
        let heard = onsets(&before);
        assert_eq!(heard.len(), 3, "{from}");
    }
}

#[test]
fn a_sample_rate_change_while_stopped_or_looping_keeps_the_position() {
    let mut rig = Rig::new(48_000);
    rig.setup(120.0, (4, 4), 1.0);
    rig.send(&[M::Play { from: Tick(500) }]);
    rig.render(1000);
    rig.send(&[M::Stop]);
    rig.render(10);
    let frozen = rig.e.clock().position();
    rig.e.set_sample_rate(SampleRate::HZ_96000);
    assert_eq!(rig.e.clock().position(), frozen);
    // Same rate again: a no-op.
    rig.e.set_sample_rate(SampleRate::HZ_96000);
    assert_eq!(rig.e.clock().position(), frozen);

    let mut rig = Rig::new(48_000);
    rig.setup(120.0, (4, 4), 1.0);
    rig.send(&[
        M::SetLoop {
            region: LoopRegion::new(Tick(0), Tick(3840)),
        },
        M::Play { from: Tick(0) },
    ]);
    rig.seconds(5.0);
    let pos = rig.e.clock().position();
    rig.e.set_sample_rate(SampleRate::HZ_44100);
    assert_eq!(rig.e.clock().position(), pos);
    rig.rate = 44_100.0;
    let found = onsets(&rig.seconds(10.0));
    // Still looping: clicks every beat, an accent every fourth.
    assert!(found.len() >= 19);
    let accents = found.iter().filter(|o| is_accent(o.1)).count();
    assert!((4..=6).contains(&accents), "{accents} accents");
}

#[test]
fn seconds_of_audio_with_changes_never_allocate() {
    let mut rig = Rig::new(48_000);
    rig.setup(120.0, (4, 4), 1.0);
    rig.send(&[
        M::SetLoop {
            region: LoopRegion::new(Tick(0), Tick(7680)),
        },
        M::SetRecording { recording: true },
        M::Play { from: Tick(0) },
    ]);
    let mut r = Feedback::default();
    for round in 0..30u32 {
        let bpm = 60.0 + f64::from(round) * 7.5;
        rig.send(&[
            M::SetTransport {
                tempo: Tempo::new(bpm).unwrap(),
                signature: TimeSignature::new(
                    3 + (round % 4) as u8,
                    [4, 8, 4, 16][round as usize % 4],
                )
                .unwrap(),
                launch_quantize: Quantize::default(),
            },
            M::SetMetronome {
                enabled: round % 5 != 3,
                volume: [1.0, 0.3, f32::NAN, 9.0, -2.0][round as usize % 5],
                count_in_bars: (round % 3) as u8,
            },
        ]);
        if round % 7 == 6 {
            rig.send(&[M::Stop]);
        }
        if round % 7 == 0 {
            rig.send(&[M::Play {
                from: Tick(i64::from(round) * 100),
            }]);
        }
        let out = rig.seconds(0.2);
        assert!(out.iter().all(|s| s.is_finite()));
        let new_rate = if round % 2 == 0 {
            SampleRate::HZ_44100
        } else {
            SampleRate::HZ_48000
        };
        assert_no_alloc(|| rig.e.set_sample_rate(new_rate));
        rig.rate = if round % 2 == 0 { 44_100.0 } else { 48_000.0 };
        assert_no_alloc(|| rig.e.report_transport(&mut r));
    }
}

#[test]
fn ticks_at_the_edges_of_the_range_never_panic() {
    for from in [
        i64::MAX,
        i64::MAX - 1,
        i64::MIN,
        i64::MIN + 5,
        i64::MAX - 2000,
    ] {
        for count_in in [0u8, 3, 255] {
            let mut rig = Rig::new(48_000);
            rig.setup(120.0, (4, 4), 1.0);
            rig.send(&[
                M::SetMetronome {
                    enabled: true,
                    volume: 1.0,
                    count_in_bars: count_in,
                },
                M::SetRecording { recording: true },
                M::Play { from: Tick(from) },
            ]);
            let out = rig.seconds(2.0);
            assert!(out.iter().all(|s| s.is_finite()));
            rig.send(&[M::Stop, M::Play { from: Tick(from) }]);
            rig.seconds(0.5);
        }
    }
}

/// Writes a short listening render for the founder (`cargo test -p engine --test
/// metronome -- --ignored listening`): 4/4 at 100 BPM, two bars at the project's default
/// level (volume 1.0), then 6/8 at 90 BPM, then a 4/4 count-in; mono 16-bit, 48 kHz.
#[test]
#[ignore]
fn listening_render() {
    let mut rig = Rig::new(48_000);
    rig.setup(100.0, (4, 4), 1.0);
    rig.send(&[M::Play { from: Tick(0) }]);
    let mut out = rig.seconds(4.8);
    rig.send(&[
        M::Stop,
        M::SetTransport {
            tempo: Tempo::new(90.0).unwrap(),
            signature: TimeSignature::new(6, 8).unwrap(),
            launch_quantize: Quantize::default(),
        },
        M::Play { from: Tick(0) },
    ]);
    out.extend(rig.seconds(4.0));
    rig.send(&[
        M::Stop,
        M::SetTransport {
            tempo: Tempo::new(120.0).unwrap(),
            signature: TimeSignature::COMMON,
            launch_quantize: Quantize::default(),
        },
        M::SetMetronome {
            enabled: true,
            volume: 1.0,
            count_in_bars: 1,
        },
        M::SetRecording { recording: true },
        M::Play { from: Tick(0) },
    ]);
    out.extend(rig.seconds(4.0));
    let data: Vec<u8> = out
        .iter()
        .flat_map(|s| ((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes())
        .collect();
    let mut wav = Vec::new();
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    for v in [16u32, 0x0001_0001, 48_000, 96_000, 0x0010_0002] {
        wav.extend_from_slice(&v.to_le_bytes());
    }
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(data.len() as u32).to_le_bytes());
    wav.extend_from_slice(&data);
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.qa/renders");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("metronome-tr2.wav"), wav).unwrap();
}

#[test]
fn no_count_in_when_the_cue_is_past_the_end_of_the_loop() {
    let mut rig = Rig::new(48_000);
    rig.setup(120.0, (4, 4), 1.0);
    rig.send(&[
        M::SetMetronome {
            enabled: true,
            volume: 1.0,
            count_in_bars: 2,
        },
        M::SetLoop {
            region: LoopRegion::new(Tick(0), Tick(3840)),
        },
        M::SetRecording { recording: true },
        M::Play { from: Tick(5000) },
    ]);
    rig.seconds(10.0);
    // It plays on past the loop from the cue, as without a count-in.
    assert!(rig.e.clock().playhead_tick().0 > 5000 + 9000);
    // A cue inside the loop still gets its count-in, and reaches the cue.
    let mut rig = Rig::new(48_000);
    rig.setup(120.0, (4, 4), 1.0);
    rig.send(&[
        M::SetMetronome {
            enabled: true,
            volume: 1.0,
            count_in_bars: 2,
        },
        M::SetLoop {
            region: LoopRegion::new(Tick(0), Tick(3840)),
        },
        M::SetRecording { recording: true },
        M::Play { from: Tick(1920) },
    ]);
    rig.render(1);
    assert!(rig.e.clock().playhead_tick().0 < 0);
    rig.seconds(4.2);
    assert!((0..3840).contains(&rig.e.clock().playhead_tick().0));
}

#[test]
fn the_click_has_the_default_peaks_and_decays_below_minus_40_dbfs_quickly() {
    let mut rig = Rig::new(48_000);
    rig.setup(120.0, (4, 4), 1.0);
    rig.send(&[M::Play { from: Tick(0) }]);
    let out = rig.seconds(2.0);
    let db = |x: f32| 20.0 * f64::from(x.abs()).log10();
    // Accent at 0 and beat at 24 000: peak at the onset sample, then below -40 dBFS
    // within 80 ms (accent) and 50 ms (beat), and silent after the click.
    for (start, peak_db, limit_ms) in [(0usize, -12.0, 80.0), (24_000, -15.0, 50.0)] {
        let peak = out[start..start + 2000]
            .iter()
            .fold(0.0f32, |m, s| m.max(s.abs()));
        assert!((db(peak) - peak_db).abs() < 0.01, "peak {} dBFS", db(peak));
        let from = start + (limit_ms * 48.0) as usize;
        assert!(
            out[from..start + 12_000]
                .iter()
                .all(|s| db(*s) < -40.0 || *s == 0.0)
        );
    }
}
