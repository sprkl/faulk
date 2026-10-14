//! Offline tests of the musical clock (TR-1): sample-accurate play, stop and locate, no
//! drift over ten minutes at 44.1, 48 and 96 kHz, loop wraps without gap or duplicate,
//! tempo changes without a jump, arbitrary orders without a panic, and no allocation on
//! the audio path.

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use engine::engine::State;
use engine::msg::TransportState;
use engine::time::{LoopRegion, SampleRate, Tempo, Tick};
use engine::transport::{Segment, Transport};
use proptest::prelude::*;

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

const RATES: [u32; 3] = [44_100, 48_000, 96_000];
const SIZES: [usize; 9] = [1, 7, 64, 100, 127, 480, 512, 1021, 4096];

struct Rig {
    t: Transport,
    s: State,
    /// Every event delivered so far: (tick, absolute sample).
    events: Vec<(i64, u64)>,
    last_to: Option<i64>,
}

impl Rig {
    fn new(rate: u32) -> Self {
        Rig {
            t: Transport::new(SampleRate::new(rate).unwrap()),
            s: State::try_new().unwrap(),
            events: Vec::new(),
            last_to: None,
        }
    }

    fn play(&mut self, from: i64) {
        self.s.playback.playing = true;
        self.s.playback.cue = Some(Tick(from));
        self.last_to = None;
    }

    fn stop(&mut self) {
        self.s.playback.playing = false;
    }

    fn seek(&mut self, tick: i64) {
        self.s.playback.cue = Some(Tick(tick));
        self.last_to = None;
    }

    fn set_loop(&mut self, start: i64, end: i64) {
        self.s.playback.loop_region = LoopRegion::new(Tick(start), Tick(end));
    }

    fn tempo(&mut self, bpm: f64) {
        self.s.transport.tempo = Tempo::new(bpm).unwrap();
    }

    /// One block; returns its segments.
    fn block(&mut self, frames: usize) -> Vec<Segment> {
        let start = self.t.total_samples();
        self.t.begin_block(&mut self.s, frames);
        let mut segments = Vec::new();
        while let Some(seg) = self.t.next_segment() {
            segments.push(seg);
            for tick in seg.from.0..seg.to.0 {
                let at = start + seg.offset_of(Tick(tick)) as u64;
                self.events.push((tick, at));
            }
        }
        self.t.end_block();
        segments
    }

    fn run(&mut self, samples: u64) {
        let mut done = 0u64;
        let mut i = 0;
        while done < samples {
            let n = (SIZES[i % SIZES.len()] as u64).min(samples - done);
            let segments = self.block(n as usize);
            self.check_tiling(&segments, n as usize);
            done += n;
            i += 1;
        }
    }

    /// The segments tile the block and each one starts where the previous one ended
    /// (except after a loop wrap, where it starts at the loop start).
    fn check_tiling(&mut self, segments: &[Segment], frames: usize) {
        if !self.t.is_running() {
            assert!(segments.is_empty());
            return;
        }
        let mut at = 0;
        for seg in segments {
            assert_eq!(seg.offset, at, "segments tile the block");
            at += seg.frames;
            assert!(seg.from <= seg.to);
            if let Some(prev) = self.last_to
                && seg.from.0 != prev
            {
                let region = self.s.playback.loop_region.expect("a jump needs a loop");
                assert_eq!(seg.from, region.start(), "a jump goes to the loop start");
                assert_eq!(prev, region.end().0, "a jump leaves from the loop end");
            }
            self.last_to = Some(seg.to.0);
        }
        assert_eq!(at, frames, "the segments cover the whole block");
    }
}

/// Exact sample of a tick at 120 BPM: `tick * rate / 1920` (a dyadic rational at these
/// rates, so the clock's `f64` is exact).
fn exact_sample(tick: i128, rate: u32) -> u64 {
    (tick * i128::from(rate) / 1920) as u64
}

#[test]
fn play_places_every_tick_on_its_exact_sample() {
    for rate in RATES {
        let mut rig = Rig::new(rate);
        rig.play(0);
        rig.run(u64::from(rate) * 3);
        assert!(!rig.events.is_empty());
        for (i, (tick, at)) in rig.events.iter().enumerate() {
            assert_eq!(*tick, i as i64, "ticks are delivered in order, once");
            assert_eq!(*at, exact_sample(i128::from(*tick), rate), "rate {rate}");
        }
    }
}

#[test]
fn no_drift_over_ten_minutes_at_120_bpm() {
    for rate in RATES {
        let mut rig = Rig::new(rate);
        rig.play(0);
        let samples = 600 * u64::from(rate);
        rig.run(samples);
        assert_eq!(rig.t.total_samples(), samples);
        // 120 BPM: 2 beats per second, 960 ticks each: 1,152,000 ticks in ten minutes.
        assert_eq!(rig.t.position(), 1_152_000.0, "rate {rate}");
        assert_eq!(rig.t.playhead_tick(), Tick(1_152_000));
        assert_eq!(rig.t.playhead_frame().get(), samples);
        assert_eq!(rig.events.len(), 1_152_000);
        for (i, (tick, at)) in rig.events.iter().enumerate() {
            assert_eq!(*tick, i as i64);
            assert_eq!(*at, exact_sample(i128::from(*tick), rate), "rate {rate}");
        }
    }
}

#[test]
fn stop_freezes_the_playhead_and_seek_moves_it() {
    let mut rig = Rig::new(48_000);
    rig.play(0);
    rig.run(48_000);
    assert_eq!(rig.t.state(), TransportState::Playing);
    rig.stop();
    rig.block(100);
    let stopped_at = rig.t.position();
    assert_eq!(rig.t.state(), TransportState::Stopped);
    assert_eq!(stopped_at, 1920.0, "frozen at the position of the stop");
    let events = rig.events.len();
    rig.run(10_000);
    assert_eq!(rig.events.len(), events, "no event while stopped");
    assert_eq!(rig.t.position(), stopped_at);
    // Locate while stopped: the playhead moves, nothing plays.
    rig.seek(9600);
    rig.block(64);
    assert_eq!(rig.t.playhead_tick(), Tick(9600));
    assert_eq!(rig.events.len(), events);
    // Play from there: the first event is on the first sample of the block.
    let start = rig.t.total_samples();
    rig.play(9600);
    rig.block(256);
    assert_eq!(rig.events[events], (9600, start));
    // Locate while playing: the delivery restarts at the cue.
    rig.seek(480);
    let start = rig.t.total_samples();
    let before = rig.events.len();
    rig.block(256);
    assert_eq!(rig.events[before], (480, start));
    assert_eq!(rig.events[before + 1], (481, start + 25));
}

#[test]
fn a_reset_state_stops_the_clock_and_playing_without_a_cue_resumes_in_place() {
    let mut rig = Rig::new(48_000);
    rig.play(960);
    rig.run(4800);
    let at = rig.t.position();
    rig.s = State::try_new().unwrap();
    rig.block(64);
    assert_eq!(rig.t.state(), TransportState::Stopped);
    assert_eq!(rig.t.position(), at);
    rig.s.playback.playing = true;
    let before = rig.events.len();
    rig.block(256);
    assert_eq!(rig.events[before].0, at.ceil() as i64);
}

/// Ticks elapsed on the clock when the loop `[start, end)`, played from tick 0, delivers
/// `tick` after `laps` wraps.
fn elapsed_in_loop(tick: i64, laps: i64, start: i64, end: i64) -> i64 {
    if laps == 0 {
        tick
    } else {
        end + (laps - 1) * (end - start) + (tick - start)
    }
}

#[test]
fn the_loop_wraps_without_gap_or_duplicate_at_every_block_size() {
    for rate in RATES {
        for size in SIZES.iter().copied().chain([3usize, 25, 26, 5000]) {
            let (start, end) = (960, 2880);
            let mut rig = Rig::new(rate);
            rig.set_loop(start, end);
            rig.play(0);
            let mut done = 0u64;
            while done < u64::from(rate) * 20 {
                let segments = rig.block(size);
                rig.check_tiling(&segments, size);
                done += size as u64;
            }
            let mut expect = 0i64;
            let mut laps = 0i64;
            for (tick, at) in &rig.events {
                assert_eq!(*tick, expect, "rate {rate} size {size}");
                assert_eq!(
                    *at,
                    exact_sample(i128::from(elapsed_in_loop(*tick, laps, start, end)), rate),
                    "rate {rate} size {size} tick {tick}"
                );
                expect += 1;
                if expect == end {
                    expect = start;
                    laps += 1;
                }
            }
            assert!(laps > 10);
        }
    }
}

#[test]
fn a_one_tick_loop_wraps_more_than_a_hundred_times_in_a_block() {
    let mut rig = Rig::new(48_000);
    rig.set_loop(500, 501);
    rig.play(500);
    let segments = rig.block(4096);
    assert!(segments.len() > 100);
    assert!(rig.events.iter().all(|(tick, _)| *tick == 500));
    assert_eq!(rig.events.len(), 4096 / 25 + 1);
    for (i, (_, at)) in rig.events.iter().enumerate() {
        assert_eq!(*at, i as u64 * 25);
    }
}

#[test]
fn a_loop_shorter_than_a_sample_is_ignored() {
    // 500 Hz: a tick lasts 0.26 samples.
    let mut rig = Rig::new(500);
    rig.set_loop(0, 1);
    rig.play(0);
    rig.block(100);
    assert!(rig.events.len() > 300);
    assert_eq!(rig.events.last().unwrap().0 as usize, rig.events.len() - 1);
}

#[test]
fn loop_changes_in_playback_never_skip_or_repeat_a_tick() {
    let mut rig = Rig::new(48_000);
    rig.play(0);
    rig.run(10_000);
    // Set the loop around the playhead.
    let now = rig.t.playhead_tick().0;
    rig.set_loop(now - 100, now + 400);
    rig.run(48_000);
    assert!(rig.events.iter().any(|(t, _)| *t == now - 100));
    // Seek beyond the loop end: plays on, no wrap.
    rig.seek(5000);
    let before = rig.events.len();
    rig.run(48_000);
    let ticks: Vec<i64> = rig.events[before..].iter().map(|e| e.0).collect();
    assert_eq!(ticks[0], 5000);
    assert!(ticks.windows(2).all(|w| w[1] == w[0] + 1));
    // Clear the loop while inside it: plays on past its end.
    rig.set_loop(6000, 7000);
    rig.seek(6500);
    rig.run(1000);
    rig.s.playback.loop_region = None;
    let before = rig.events.len();
    rig.run(96_000);
    let ticks: Vec<i64> = rig.events[before..].iter().map(|e| e.0).collect();
    assert!(ticks.windows(2).all(|w| w[1] == w[0] + 1));
    assert!(*ticks.last().unwrap() >= 7000, "ran past the old loop end");
}

#[test]
fn a_tempo_change_in_playback_does_not_jump() {
    for rate in RATES {
        let mut rig = Rig::new(rate);
        rig.play(0);
        let mut expected = 0.0f64;
        let plan = [
            (120.0, 3),
            (90.0, 7),
            (240.0, 11),
            (20.0, 2),
            (999.0, 13),
            (60.1, 5),
        ];
        for (bpm, blocks) in plan {
            let before = rig.t.position();
            rig.tempo(bpm);
            rig.block(480);
            // The block that applies the change starts at the position the old tempo
            // reached (the clock re-anchors there) and moves at the new tempo.
            let per_block = 480.0 * 960.0 * bpm / (60.0 * f64::from(rate));
            let moved = rig.t.position() - before;
            assert!((moved - per_block).abs() < 1e-6, "{moved} vs {per_block}");
            expected += per_block;
            for _ in 1..blocks {
                rig.block(480);
                expected += per_block;
            }
        }
        assert!((rig.t.position() - expected).abs() < 1e-5, "rate {rate}");
        // Every tick exactly once, in order, across all the changes.
        for (i, (tick, _)) in rig.events.iter().enumerate() {
            assert_eq!(*tick, i as i64);
        }
        assert!(rig.events.windows(2).all(|w| w[0].1 <= w[1].1));
    }
}

#[test]
fn a_tempo_change_while_stopped_applies_at_play() {
    let mut rig = Rig::new(48_000);
    rig.tempo(60.0);
    rig.block(64);
    rig.play(0);
    rig.block(100);
    // 60 BPM at 48 kHz: 50 samples per tick.
    assert_eq!(
        rig.events[1],
        (1, 64 + 50),
        "the play block starts at sample 64"
    );
}

#[test]
fn zero_frame_blocks_and_extreme_positions_do_not_panic() {
    let mut rig = Rig::new(48_000);
    rig.play(i64::MAX - 10);
    rig.block(0);
    rig.block(4096);
    assert_eq!(rig.t.playhead_tick(), Tick(i64::MAX));
    rig.seek(i64::MIN);
    rig.block(100);
    assert_eq!(
        rig.t.playhead_frame().get(),
        0,
        "negative positions give frame 0"
    );
    rig.set_loop(i64::MIN, i64::MAX);
    rig.block(100);
    rig.set_loop(i64::MAX - 5, i64::MAX);
    rig.seek(i64::MAX - 5);
    rig.block(4096);
}

#[test]
fn the_audio_path_does_not_allocate() {
    let mut rig = Rig::new(48_000);
    rig.set_loop(0, 1920);
    rig.s.playback.recording = true;
    rig.play(0);
    let mut seen = 0u64;
    for i in 0..2000usize {
        let frames = SIZES[i % SIZES.len()];
        match i % 400 {
            50 => rig.s.transport.tempo = Tempo::new(97.0).unwrap(),
            100 => rig.s.playback.cue = Some(Tick(300)),
            150 => rig.s.playback.playing = false,
            200 => {
                rig.s.playback.playing = true;
                rig.s.playback.cue = Some(Tick(0));
            }
            250 => rig.s.transport.tempo = Tempo::new(180.0).unwrap(),
            300 => rig.s.playback.loop_region = None,
            350 => rig.s.playback.loop_region = LoopRegion::new(Tick(100), Tick(700)),
            _ => {}
        }
        assert_no_alloc(|| {
            rig.t.begin_block(&mut rig.s, frames);
            while let Some(seg) = rig.t.next_segment() {
                seen += (seg.to.0 - seg.from.0) as u64;
            }
            rig.t.end_block();
        });
    }
    assert!(seen > 10_000);
    assert_eq!(rig.t.state(), TransportState::Recording);
}

#[derive(Clone, Debug)]
enum Order {
    Play(i64),
    Stop,
    Seek(i64),
    Loop(i64, i64),
    NoLoop,
    Tempo(f64),
    Block(usize),
}

fn orders() -> impl Strategy<Value = Vec<Order>> {
    let tick = prop_oneof![
        -5000i64..20_000,
        Just(i64::MIN),
        Just(i64::MAX),
        any::<i64>()
    ];
    let order = prop_oneof![
        tick.clone().prop_map(Order::Play),
        Just(Order::Stop),
        tick.clone().prop_map(Order::Seek),
        (tick.clone(), tick).prop_map(|(a, b)| Order::Loop(a, b)),
        Just(Order::NoLoop),
        (20.0f64..=999.0).prop_map(Order::Tempo),
        (0usize..2000).prop_map(Order::Block),
        (0usize..2000).prop_map(Order::Block),
        (0usize..2000).prop_map(Order::Block),
    ];
    prop::collection::vec(order, 1..80)
}

proptest! {
    #[test]
    fn arbitrary_orders_never_panic_and_segments_tile_every_block(
        rate in prop::sample::select(vec![8_000u32, 44_100, 48_000, 96_000, 192_000]),
        script in orders(),
    ) {
        let mut rig = Rig::new(rate);
        for order in script {
            match order {
                Order::Play(t) => rig.play(t),
                Order::Stop => rig.stop(),
                Order::Seek(t) => rig.seek(t),
                Order::Loop(a, b) => {
                    if let Some(region) = LoopRegion::new(Tick(a), Tick(b)) {
                        rig.s.playback.loop_region = Some(region);
                    }
                }
                Order::NoLoop => rig.s.playback.loop_region = None,
                Order::Tempo(bpm) => rig.tempo(bpm),
                Order::Block(n) => {
                    let segments = rig.block(n);
                    let mut at = 0;
                    for seg in &segments {
                        prop_assert_eq!(seg.offset, at);
                        at += seg.frames;
                        prop_assert!(seg.from <= seg.to);
                    }
                    if rig.t.is_running() {
                        prop_assert_eq!(at, n);
                    } else {
                        prop_assert!(segments.is_empty());
                    }
                    prop_assert!(rig.t.position().is_finite());
                }
            }
        }
    }
}

#[test]
fn a_seam_on_a_block_boundary_or_in_the_last_sample_keeps_the_loop() {
    // 48 kHz, 120 BPM, one bar: the wrap sample 96000 is the end of block 749 of 128.
    for (rate, size) in [(48_000, 128), (44_100, 128), (44_100, 1), (96_000, 128)] {
        let mut rig = Rig::new(rate);
        rig.set_loop(0, 3840);
        rig.play(0);
        let mut done = 0u64;
        while done < u64::from(rate) * 30 {
            let segments = rig.block(size);
            rig.check_tiling(&segments, size);
            done += size as u64;
        }
        let max = rig.events.iter().map(|e| e.0).max().unwrap();
        assert_eq!(
            max, 3839,
            "rate {rate} size {size}: the playhead stays in the loop"
        );
        assert!(rig.events.len() > 3840 * 10);
    }
}

#[test]
fn a_tempo_change_just_before_the_loop_end_keeps_the_loop() {
    for rate in RATES {
        for offset in [1i64, 2, 3, 10, 100] {
            let mut rig = Rig::new(rate);
            rig.set_loop(0, 1000);
            rig.play(1000 - offset);
            rig.block(1);
            rig.tempo(121.7);
            rig.run(u64::from(rate) * 5);
            let max = rig.events.iter().map(|e| e.0).max().unwrap();
            assert_eq!(max, 999, "rate {rate} offset {offset}");
        }
    }
}

#[test]
fn a_loop_at_a_non_dyadic_tempo_stays_on_the_closed_form_over_many_wraps() {
    let rate = 44_100u32;
    let bpm = 133.5;
    let mut rig = Rig::new(rate);
    rig.tempo(bpm);
    rig.set_loop(0, 4);
    rig.play(0);
    rig.run(u64::from(rate) * 120);
    let spt = 60.0 * f64::from(rate) / (bpm * 960.0);
    let laps = rig.events.len() as i64 / 4;
    assert!(laps > 50_000, "{laps} wraps");
    for (i, (tick, at)) in rig.events.iter().enumerate() {
        assert_eq!(*tick, (i % 4) as i64);
        let exact = (i as f64 * spt).floor() as i64;
        // The loop start is clamped into the first sample of its segment (up to one sample
        // late) and a sample boundary can tip by the rounding noise of the anchors.
        assert!(
            (*at as i64 - exact).abs() <= 2,
            "event {i}: {at} vs {exact}"
        );
    }
}

#[test]
fn a_loop_that_rounds_to_nothing_in_f64_never_hangs() {
    // Beyond 2^53 ticks the start and the end of a 5-tick loop are the same f64.
    let mut rig = Rig::new(48_000);
    let base = 1i64 << 60;
    rig.set_loop(base, base + 5);
    rig.play(base - 1000);
    rig.block(4096);
    rig.block(4096);
    assert!(rig.t.is_running());
    // A loop long enough for f64 still wraps up there.
    rig.set_loop(base, base + 1_000_000);
    rig.seek(base + 999_000);
    rig.block(48_000);
}

#[test]
fn a_loop_that_turns_on_and_off_with_the_tempo_neither_repeats_nor_hangs() {
    // 8 kHz: a one tick loop is longer than a sample only at slow tempos.
    let mut rig = Rig::new(8_000);
    rig.set_loop(0, 1);
    rig.tempo(120.0);
    rig.play(0);
    rig.block(100);
    rig.tempo(999.0);
    rig.run(8_000);
    rig.tempo(120.0);
    let before = rig.events.len();
    rig.block(4096);
    // Playing on past the loop end: strictly increasing ticks, no repeat of tick 0.
    let ticks: Vec<i64> = rig.events[before..].iter().map(|e| e.0).collect();
    assert!(ticks.windows(2).all(|w| w[1] == w[0] + 1), "{ticks:?}");
}

#[test]
fn tempo_changes_near_loop_seams_deliver_every_tick_exactly_once_in_order() {
    for rate in RATES {
        for size in [1usize, 3, 64, 128, 480, 1021] {
            let mut rig = Rig::new(rate);
            rig.set_loop(0, 480);
            rig.play(0);
            let mut done = 0u64;
            let mut i = 0;
            while done < u64::from(rate) * 20 {
                if i % 37 == 0 {
                    rig.tempo(60.0 + (i % 11) as f64 * 37.3);
                }
                let segments = rig.block(size);
                rig.check_tiling(&segments, size);
                done += size as u64;
                i += 1;
            }
            let mut expect = 0;
            for (tick, _) in &rig.events {
                assert_eq!(*tick, expect, "rate {rate} size {size}");
                expect = (expect + 1) % 480;
            }
            assert!(rig.events.len() > 480 * 10);
        }
    }
}
