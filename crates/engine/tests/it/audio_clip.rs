//! AU-4: the audio clip player ([`engine::devices::clip_player`]) through the real channel and
//! a disk held in memory ([`engine::audio_source::scenario`]): a reference clip rendered
//! sample for sample, resampling (pitch and duration measured), sample-exact onsets, clicks
//! measured at launch, stop, loop wrap, switch, jump and underrun, the end of a clip played
//! once, an offline source, a rate change and the absence of allocation on the audio side.

use std::sync::atomic::Ordering;

use assert_no_alloc::assert_no_alloc;
use engine::audio_source::FileKind;
use engine::audio_source::scenario::{
    BLOCK, CLIP_START, LONG, LONG_LOOP, LONG_LOOP_CLIP, MemDisk, RATE, Rig, SHORT_GAIN, launch,
    long_value, placement, player_setup, short_value, wav16,
};
use engine::devices::clip_player::fade_frames;
use engine::msg::{
    ClipSlot, ControlMsg, Placement, PlacementSlot, SceneSlot, SourceSlot, Span, TrackKind,
    TrackSlot,
};
use engine::time::{Frame, PPQ, Quantize, SampleRate, Tempo, Tick, TimeSignature};

/// The player's fade in (2 ms) and its gate's: the frames after an onset that may differ from
/// the source. A track that exists before the onset adds no fade of its own: its strip took
/// its levels while it rendered nothing (#759).
fn settle_frames(rate: u32) -> usize {
    2 * fade_frames(rate) as usize
}

fn underruns(rig: &Rig) -> u32 {
    rig.host
        .engine()
        .disk_stats()
        .underruns
        .load(Ordering::Relaxed)
}

fn first_sound(left: &[f32]) -> Option<usize> {
    left.iter().position(|s| *s != 0.0)
}

/// The largest step between two neighbouring samples of either channel.
fn max_step(left: &[f32], right: &[f32]) -> f32 {
    let step = |x: &[f32]| {
        x.windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0, f32::max)
    };
    step(left).max(step(right))
}

fn transport(quantize: Quantize, bpm: f64) -> ControlMsg {
    ControlMsg::SetTransport {
        tempo: Tempo::new(bpm).unwrap(),
        signature: TimeSignature::default(),
        launch_quantize: quantize,
    }
}

fn standard() -> Rig {
    let mut rig = Rig::new(RATE, MemDisk::standard(), &player_setup());
    rig.render(200);
    rig
}

#[test]
fn a_clip_launched_from_the_grid_plays_its_source_sample_for_sample() {
    let mut rig = standard();
    rig.send(&[ControlMsg::Play { from: Tick(0) }, launch(0)]);
    // 32 000 frames: past the 1.5 s preload, through the ring.
    let (l, r) = rig.render(250);
    // Launched at once: the clip's first frame is the block's first (the player's fade in,
    // from 0, makes that frame silent).
    let onset = 0;
    assert!(first_sound(&l).is_some_and(|i| i <= 1), "it starts at once");
    let skip = settle_frames(RATE);
    let mut compared = 0;
    for i in onset + skip..l.len() {
        let f = CLIP_START + (i - onset) as u64;
        assert_eq!(
            [l[i], r[i]],
            [long_value(f, 0), long_value(f, 1)],
            "frame {i}"
        );
        compared += 1;
    }
    assert!(compared > 30_000);
    assert_eq!(underruns(&rig), 0);
    let playing = rig
        .host
        .engine()
        .state()
        .track(TrackSlot(0))
        .unwrap()
        .player
        .playing();
    assert_eq!(
        playing.map(|p| p.clip),
        Some(ClipSlot(0)),
        "the grid reports it"
    );
}

/// An Echo with no feedback nor wet signal on track 0: its output is its input, bit for bit,
/// but it keeps the track rendered while the track waits (#759).
fn transparent_echo() -> ControlMsg {
    use engine::devices::echo::{DESCRIPTOR, FEEDBACK, MIX};
    use engine::msg::{DeviceSlot, ParamId, encode_state};
    ControlMsg::DeviceInsert {
        track: TrackSlot(0),
        device: DeviceSlot(0),
        kind: DESCRIPTOR.kind_id,
        state: encode_state(&[(ParamId(FEEDBACK), 0.0), (ParamId(MIX), 0.0)]),
    }
}

/// The first 20 blocks of a 1 kHz sine launched with the `Play` on a 48 kHz rig (a fade of a
/// strip, 240 frames, is longer than a block), `gap` stopped blocks after `before`.
fn launch_after(before: &[ControlMsg], gap: usize) -> (Vec<f32>, Vec<f32>) {
    let (mut rig, _) = sine_rig(48_000, 48_000, 1_000.0, 2.0, None);
    rig.render(200);
    rig.send(before);
    rig.render(gap);
    rig.send(&[ControlMsg::Play { from: Tick(0) }, launch(0)]);
    rig.render(20)
}

#[test]
fn a_transparent_effect_on_an_audio_track_leaves_its_launch_as_it_is() {
    // #759: with or without the Echo, the clip's start is the same, sample for sample, and
    // so is a fader moved shortly before the launch, its ramp included.
    let gain = ControlMsg::TrackSetMix {
        track: TrackSlot(0),
        gain: 0.5,
        pan: 0.0,
        mute: false,
    };
    let plain = launch_after(&[], 0);
    assert!(plain.0.iter().any(|s| *s != 0.0));
    assert_eq!(launch_after(&[transparent_echo()], 0), plain);
    assert_eq!(launch_after(&[transparent_echo()], 10), plain);
    for gap in [0, 1, 2, 10] {
        let faded = launch_after(std::slice::from_ref(&gain), gap);
        assert_eq!(
            launch_after(&[gain.clone(), transparent_echo()], gap),
            faded,
            "a fader moved {gap} blocks before the launch"
        );
        assert_ne!(faded, plain);
    }
    // Moved 10 blocks before (past its 5 ms ramp), the fader is the clip's level from its
    // first frame past the player's fade.
    let (l, _) = launch_after(&[gain], 10);
    let (p, _) = &plain;
    let skip = 2 * fade_frames(48_000) as usize;
    for i in skip..l.len() {
        assert_eq!(l[i], 0.5 * p[i], "frame {i}");
    }
}

#[test]
fn a_looped_clip_plays_at_its_gain_and_wraps_on_time() {
    let mut rig = standard();
    rig.send(&[ControlMsg::Play { from: Tick(0) }, launch(1)]);
    let (l, _) = rig.render(100);
    let onset = 0;
    let edge = fade_frames(RATE) as usize;
    let at = |k: usize| {
        if k < 1_800 {
            k
        } else {
            1_000 + (k - 1_800) % 800
        }
    };
    for (k, s) in l.iter().enumerate().skip(onset) {
        let k = k - onset;
        // Away from the onset and from each wrap's declick.
        if k < settle_frames(RATE) || (k >= 1_800 && (k - 1_800) % 800 < edge) {
            continue;
        }
        assert_eq!(
            *s,
            short_value(at(k) as u64, 0) * SHORT_GAIN,
            "frame {k} of the clip"
        );
    }
}

/// A mono sine of `hz` at `rate`, `secs` long, at half scale.
fn sine_disk(media: [u8; 32], rate: u32, hz: f64, secs: f64) -> (MemDisk, u64) {
    let frames = (f64::from(rate) * secs) as u64;
    let wav = wav16(rate, 1, frames, |f, _| {
        ((f as f64 * hz * core::f64::consts::TAU / f64::from(rate)).sin() * 16_384.0) as i16
    });
    let mut disk = MemDisk::default();
    disk.files.insert((media, FileKind::Blob as u8), wav);
    (disk, frames)
}

/// A rig at `engine` Hz whose track 0 plays clip 0, the whole sine source (mono, `rate` Hz),
/// from grid scene 0, `looping` or once.
fn sine_rig(engine: u32, rate: u32, hz: f64, secs: f64, looping: Option<(u64, u64)>) -> (Rig, u64) {
    let media = [0x5a; 32];
    let (disk, frames) = sine_disk(media, rate, hz, secs);
    let setup = [
        transport(Quantize::None, 120.0),
        ControlMsg::TrackCreate {
            track: TrackSlot(0),
            kind: TrackKind::Audio,
        },
        ControlMsg::AudioSourceDeclare {
            source: SourceSlot(0),
            sample_rate: rate,
            channels: 1,
            frames: Frame(frames),
            media,
        },
        ControlMsg::ClipAudioSet {
            clip: ClipSlot(0),
            source: SourceSlot(0),
            start: Frame(0),
            length: Frame(frames),
            gain: 1.0,
            looping: looping.map(|(a, b)| (Frame(a), Frame(b))),
        },
        ControlMsg::SceneSetSlot {
            track: TrackSlot(0),
            scene: SceneSlot(0),
            clip: Some(ClipSlot(0)),
        },
    ];
    let mut rig = Rig::new(engine, disk, &setup);
    rig.render(100);
    (rig, frames)
}

/// The frequency of a steady sine: rising zero crossings over the frames between the first
/// and the last.
fn frequency(x: &[f32], rate: u32) -> f64 {
    let ups: Vec<usize> = x
        .windows(2)
        .enumerate()
        .filter(|(_, w)| w[0] < 0.0 && w[1] >= 0.0)
        .map(|(i, _)| i)
        .collect();
    let (first, last) = (ups[0], *ups.last().unwrap());
    (ups.len() - 1) as f64 * f64::from(rate) / (last - first) as f64
}

#[test]
fn a_clip_resampled_keeps_its_pitch_and_its_duration() {
    for (engine, rate) in [(48_000, 44_100), (44_100, 48_000), (48_000, 48_000)] {
        let (mut rig, frames) = sine_rig(engine, rate, 1_000.0, 1.0, None);
        rig.send(&[ControlMsg::Play { from: Tick(0) }, launch(0)]);
        let (l, _) = rig.render((2 * engine as usize).div_ceil(BLOCK));
        let onset = first_sound(&l).unwrap();
        let end = l.iter().rposition(|s| *s != 0.0).unwrap();
        let heard = end - onset + 1;
        let expected = frames as f64 * f64::from(engine) / f64::from(rate);
        let steady = &l[onset + 200..end - 200];
        let hz = frequency(steady, engine);
        println!(
            "source {rate} Hz in an engine at {engine} Hz: {hz:.3} Hz (1000 expected), \
             {heard} frames heard ({expected:.1} expected)"
        );
        assert!((hz - 1_000.0).abs() < 1.0, "{hz} Hz");
        assert!(
            (heard as f64 - expected).abs() <= 2.0,
            "{heard} frames, {expected} expected"
        );
        // The grid lets it go once its last frame played.
        assert_eq!(
            rig.host
                .engine()
                .state()
                .track(TrackSlot(0))
                .unwrap()
                .player
                .playing(),
            None
        );
    }
}

#[test]
fn a_launch_on_a_bar_starts_on_the_floor_of_its_sample() {
    // 130 BPM at 8 kHz: 3.846… samples a tick, a bar's sample is not whole.
    let mut setup = player_setup();
    setup[0] = transport(Quantize::Bar, 130.0);
    let mut rig = Rig::new(RATE, MemDisk::standard(), &setup);
    rig.render(200);
    rig.send(&[ControlMsg::Play { from: Tick(0) }]);
    let start = rig.frames;
    rig.render(10);
    rig.send(&[launch(0)]);
    let (l, _) = rig.render(200);
    let onset = first_sound(&l).unwrap() as u64 + start + 10 * BLOCK as u64;
    let bar = 4 * PPQ;
    let spt = 60.0 * f64::from(RATE) / (130.0 * PPQ as f64);
    let expected = start + (bar as f64 * spt).floor() as u64;
    assert_eq!(onset, expected, "the second bar starts at frame {expected}");
}

#[test]
fn a_placement_starts_on_the_floor_of_its_tick_s_sample() {
    let mut setup = player_setup();
    setup[0] = transport(Quantize::None, 130.0);
    let tick = Tick(1_234);
    setup.push(ControlMsg::TrackPlacementsUpload {
        track: TrackSlot(0),
        first: 0,
        total: 1,
        placements: vec![Placement {
            placement: PlacementSlot(0),
            clip: ClipSlot(0),
            start: tick,
            span: Span::Frames {
                offset: Frame(0),
                length: Frame(30 * RATE as u64),
            },
        }],
    });
    let mut rig = Rig::new(RATE, MemDisk::standard(), &setup);
    rig.render(200);
    rig.send(&[ControlMsg::Play { from: Tick(0) }]);
    let start = rig.frames;
    let (l, r) = rig.render(200);
    let spt = 60.0 * f64::from(RATE) / (130.0 * PPQ as f64);
    let expected = (tick.0 as f64 * spt).floor() as usize;
    assert_eq!(first_sound(&l), Some(expected), "from frame {start}");
    // And it plays the clip from its start.
    let i = expected + settle_frames(RATE);
    let f = CLIP_START + settle_frames(RATE) as u64;
    assert_eq!([l[i], r[i]], [long_value(f, 0), long_value(f, 1)]);
}

#[test]
fn launches_stops_wraps_switches_jumps_and_underruns_make_no_click() {
    // A 200 Hz sine at 48 kHz, 40 s (streamed, not resident), looped on a span that does not
    // hold whole periods: every wrap is a step in the source.
    let engine = 48_000;
    let (mut rig, frames) = sine_rig(engine, engine, 200.0, 40.0, Some((24_000, 24_000 + 7_777)));
    let source_step = (200.0 * core::f64::consts::TAU / f64::from(engine) * 0.5) as f32;
    let bound = source_step + 1.0 / fade_frames(engine) as f32;
    let mut l = Vec::new();
    let mut r = Vec::new();
    let mut take = |rig: &mut Rig, blocks: usize| {
        let (a, b) = rig.render(blocks);
        l.extend(a);
        r.extend(b);
    };
    // Launch, many wraps.
    rig.send(&[ControlMsg::Play { from: Tick(0) }, launch(0)]);
    take(&mut rig, 400);
    // A jump of the transport: the clip goes on from another phase (a crossfade).
    rig.send(&[ControlMsg::Seek { tick: Tick(777) }]);
    take(&mut rig, 100);
    // To the arrangement: a placement of the whole source (clip 1, no loop) entered 20 s in,
    // through the ring (a late disk).
    rig.send(&[
        ControlMsg::ClipAudioSet {
            clip: ClipSlot(1),
            source: SourceSlot(0),
            start: Frame(0),
            length: Frame(frames),
            gain: 1.0,
            looping: None,
        },
        ControlMsg::TrackPlacementsUpload {
            track: TrackSlot(0),
            first: 0,
            total: 1,
            placements: vec![Placement {
                placement: PlacementSlot(0),
                clip: ClipSlot(1),
                start: Tick(0),
                span: Span::Frames {
                    offset: Frame(0),
                    length: Frame(frames),
                },
            }],
        },
        ControlMsg::BackToArrangement,
        ControlMsg::Seek {
            tick: Tick(40 * PPQ),
        },
    ]);
    take(&mut rig, 200);
    // A disk that hangs: the ring runs dry (an underrun), then the disk comes back.
    rig.disk.stall();
    // Longer than the ring's 2.5 s ahead.
    take(&mut rig, 1_500);
    rig.disk.unstall();
    take(&mut rig, 200);
    // Back to the grid, then a stop.
    rig.send(&[launch(0)]);
    take(&mut rig, 100);
    rig.send(&[ControlMsg::StopTrack {
        track: TrackSlot(0),
    }]);
    take(&mut rig, 50);
    rig.send(&[launch(0)]);
    take(&mut rig, 50);
    rig.send(&[ControlMsg::Stop]);
    take(&mut rig, 50);
    assert!(underruns(&rig) > 0, "the hung disk was an underrun");
    let step = max_step(&l, &r);
    println!("largest step {step:.5} (source {source_step:.5}, bound {bound:.5})");
    assert!(step <= bound, "a click: step {step} > {bound}");
    assert!(l.iter().any(|s| *s != 0.0));
}

#[test]
fn a_placement_that_disappears_while_it_plays_fades_out_to_silence() {
    // `arrangement.delete` of an audio placement sends the track's table again without it.
    let engine = 48_000;
    let (mut rig, frames) = sine_rig(engine, engine, 200.0, 10.0, None);
    let source_step = (200.0 * core::f64::consts::TAU / f64::from(engine) * 0.5) as f32;
    let bound = source_step + 1.0 / fade_frames(engine) as f32;
    let table = |placements: Vec<Placement>| ControlMsg::TrackPlacementsUpload {
        track: TrackSlot(0),
        first: 0,
        total: placements.len() as u32,
        placements,
    };
    rig.send(&[
        table(vec![Placement {
            placement: PlacementSlot(0),
            clip: ClipSlot(0),
            start: Tick(0),
            span: Span::Frames {
                offset: Frame(0),
                length: Frame(frames),
            },
        }]),
        ControlMsg::Play { from: Tick(0) },
    ]);
    let (mut l, mut r) = rig.render(100);
    assert!(l[l.len() - 500..].iter().any(|s| *s != 0.0), "it plays");
    rig.send(&[table(vec![])]);
    let (a, b) = rig.render(100);
    l.extend(a);
    r.extend(b);
    let n = l.len();
    assert!(
        l[n - 500..].iter().chain(&r[n - 500..]).all(|s| *s == 0.0),
        "silent once its placement is gone"
    );
    let step = max_step(&l, &r);
    assert!(step <= bound, "a click: step {step} > {bound}");
    // Back with it: the undo plays again.
    rig.send(&[table(vec![Placement {
        placement: PlacementSlot(0),
        clip: ClipSlot(0),
        start: Tick(0),
        span: Span::Frames {
            offset: Frame(0),
            length: Frame(frames),
        },
    }])]);
    let (a, _) = rig.render(100);
    assert!(
        a[a.len() - 500..].iter().any(|s| *s != 0.0),
        "it plays again"
    );
}

#[test]
fn three_changes_within_one_fade_make_no_click() {
    let engine = 48_000;
    let (mut rig, _) = sine_rig(engine, engine, 200.0, 5.0, Some((0, 48_000)));
    let source_step = (200.0 * core::f64::consts::TAU / f64::from(engine) * 0.5) as f32;
    let bound = source_step + 1.0 / fade_frames(engine) as f32;
    rig.send(&[ControlMsg::Play { from: Tick(0) }, launch(0)]);
    let (mut l, mut r) = rig.render(50);
    for tick in [100, 300, 500, 700] {
        rig.send(&[ControlMsg::Seek { tick: Tick(tick) }]);
        // 32 frames: four seeks within one 96-frame fade.
        let n = rig.host.render(32);
        l.extend_from_slice(&rig.host.left()[..n]);
        r.extend_from_slice(&rig.host.right()[..n]);
    }
    let (a, b) = rig.render(50);
    l.extend(a);
    r.extend(b);
    let step = max_step(&l, &r);
    assert!(step <= bound, "a click: step {step} > {bound}");
    assert!(
        l[l.len() - 1000..].iter().any(|s| *s != 0.0),
        "the last seek plays"
    );
}

#[test]
fn a_clip_played_once_ends_with_its_last_frame_whatever_the_tempo() {
    let (mut rig, frames) = sine_rig(RATE, RATE, 200.0, 1.0, None);
    rig.send(&[ControlMsg::Play { from: Tick(0) }, launch(0)]);
    rig.render(20);
    // Faster: a tick end computed at this tempo would come early.
    rig.send(&[transport(Quantize::None, 240.0)]);
    let (l, _) = rig.render(60);
    let end = l.iter().rposition(|s| *s != 0.0).unwrap() + 20 * BLOCK;
    assert!(
        (end as i64 - frames as i64).unsigned_abs() <= BLOCK as u64,
        "it played to its last frame: {end} of {frames}"
    );
    assert_eq!(
        rig.host
            .engine()
            .state()
            .track(TrackSlot(0))
            .unwrap()
            .player
            .playing(),
        None
    );
}

#[test]
fn a_clip_whose_source_is_offline_is_silent_and_counted() {
    let mut disk = MemDisk::standard();
    disk.files.remove(&(LONG, FileKind::Blob as u8));
    let mut rig = Rig::new(RATE, disk, &player_setup());
    rig.render(200);
    rig.send(&[ControlMsg::Play { from: Tick(0) }, launch(0)]);
    let (l, r) = rig.render(100);
    assert!(l.iter().chain(&r).all(|s| *s == 0.0), "silent");
    rig.host.write_feedback();
    let stats = rig.host.engine().disk_stats();
    assert_eq!(
        stats.offline.load(Ordering::Relaxed),
        1,
        "the missing blob is counted"
    );
    assert_eq!(underruns(&rig), 0, "an offline source is not a slow disk");
}

#[test]
fn a_new_engine_rate_keeps_the_pitch() {
    let (mut rig, _) = sine_rig(48_000, 48_000, 1_000.0, 10.0, Some((0, 480_000)));
    rig.send(&[ControlMsg::Play { from: Tick(0) }, launch(0)]);
    rig.render(100);
    rig.prepare.set_sample_rate(SampleRate::HZ_44100).unwrap();
    rig.rate = 44_100;
    let (l, _) = rig.render(400);
    let hz = frequency(&l[2_000..], 44_100);
    println!("after the rate change: {hz:.3} Hz");
    assert!((hz - 1_000.0).abs() < 1.0, "{hz} Hz");
    assert!(l.iter().all(|s| s.is_finite()));
}

#[test]
fn the_audio_side_never_allocates_while_clips_play() {
    let mut rig = standard();
    // Clip 0 split in two at 2 s (#704): the right part takes the stream over.
    let split = ControlMsg::TrackPlacementsUpload {
        track: TrackSlot(0),
        first: 0,
        total: 2,
        placements: vec![
            part(0, 0, 0, 0, 2 * RATE as u64),
            part(1, 0, 4 * PPQ, 2 * RATE as u64, 28 * RATE as u64),
        ],
    };
    let steps: [&[ControlMsg]; 8] = [
        &[ControlMsg::Play { from: Tick(0) }, launch(0)],
        &[launch(1)],
        &[launch(2)],
        &[
            placement(),
            ControlMsg::BackToArrangement,
            ControlMsg::Seek {
                tick: Tick(30 * PPQ),
            },
        ],
        &[ControlMsg::Seek { tick: Tick(PPQ) }],
        &[
            split,
            ControlMsg::Seek {
                tick: Tick(3 * PPQ + PPQ / 2),
            },
        ],
        &[ControlMsg::StopTrack {
            track: TrackSlot(0),
        }],
        &[ControlMsg::Stop],
    ];
    for (i, step) in steps.into_iter().enumerate() {
        rig.send(step);
        for _ in 0..60 {
            assert_no_alloc(|| {
                rig.host.render(BLOCK);
                rig.host.write_feedback();
            });
            rig.frames += BLOCK as u64;
            rig.disk
                .step(&mut rig.prepare, rig.frames * 1000 / u64::from(RATE));
            rig.prepare.flush();
        }
        if i == 5 {
            // The split's step played through the cut (clip frame 16 000) under the detector.
            let at = rig
                .host
                .engine()
                .state()
                .track(TrackSlot(0))
                .unwrap()
                .audio
                .position();
            assert!(
                at.is_some_and(|f| f > CLIP_START + 2 * RATE as u64 + 1_000),
                "past the cut: {at:?}"
            );
        }
    }
    // A new rate while a clip plays.
    rig.send(&[ControlMsg::Play { from: Tick(0) }, launch(2)]);
    rig.render(10);
    rig.prepare
        .set_sample_rate(SampleRate::new(11_025).unwrap())
        .unwrap();
    for _ in 0..30 {
        assert_no_alloc(|| {
            rig.host.render(BLOCK);
        });
        rig.prepare.flush();
    }
    let _ = (LONG_LOOP, LONG_LOOP_CLIP);
}

#[test]
fn the_grid_reports_an_audio_clip_queued_then_playing_and_refuses_it_on_a_midi_track() {
    use engine::msg::ClipState;
    let mut setup = player_setup();
    setup[0] = transport(Quantize::Bar, 120.0);
    // Track 1, a MIDI track, with clip 1 (audio) in its scene 0.
    setup.push(ControlMsg::TrackCreate {
        track: TrackSlot(1),
        kind: TrackKind::Midi,
    });
    setup.push(ControlMsg::SceneSetSlot {
        track: TrackSlot(1),
        scene: SceneSlot(0),
        clip: Some(ClipSlot(1)),
    });
    let mut rig = Rig::new(RATE, MemDisk::standard(), &setup);
    rig.render(200);
    rig.send(&[ControlMsg::Play { from: Tick(0) }]);
    rig.render(10);
    rig.send(&[
        launch(0),
        ControlMsg::LaunchClip {
            track: TrackSlot(1),
            scene: SceneSlot(0),
        },
    ]);
    let report = |rig: &Rig| {
        let mut out = Vec::new();
        engine::launcher::active_clips(rig.host.engine().state(), true, |a| {
            out.push((a.track.0, a.scene.0, a.state));
        });
        out
    };
    rig.render(1);
    assert_eq!(
        report(&rig),
        vec![(0, 0, ClipState::Queued)],
        "the MIDI track's launch leaves it silent"
    );
    // Past the bar (2 s at 120 BPM).
    rig.render(130);
    assert_eq!(
        report(&rig),
        vec![
            (0, 0, ClipState::Playing),
            (1, engine::msg::HELD_SCENE.0, ClipState::Held)
        ]
    );
    let (l, _) = rig.render(1);
    assert!(l.iter().any(|s| *s != 0.0));
}

#[test]
fn the_parity_scenario_plays_every_part_without_an_underrun() {
    let out = engine::audio_source::scenario::player_scenario();
    let underruns = *out.last().unwrap();
    assert_eq!(underruns, 0.0);
    // Each part sounds: grid clip 0, clip 1, the placement after its wait, clip 2.
    let frames = (out.len() - 1) / 2;
    let part = |from: usize, blocks: usize| {
        out[2 * from * BLOCK..2 * (from + blocks) * BLOCK]
            .iter()
            .filter(|s| **s != 0.0)
            .count()
    };
    assert!(part(0, 250) > 60_000);
    assert!(part(250, 100) > 20_000);
    assert!(part(350, 150) > 30_000);
    assert!(part(500, 250) > 60_000);
    assert_eq!(frames, 1_370 * BLOCK);
    // The stop: silent at its end.
    assert!(
        out[2 * 760 * BLOCK..2 * 770 * BLOCK]
            .iter()
            .all(|s| *s == 0.0)
    );
    // The resampled part (an engine at 11 025 Hz): both clips sound.
    assert!(part(770, 100) > 20_000);
    assert!(part(870, 200) > 40_000);
    // The split placements (#704), at 8 000, 11 025 and 6 000 Hz: they play through.
    for from in [1_070, 1_170, 1_270] {
        assert!(
            part(from, 100) > 20_000,
            "the split placements from block {from}"
        );
    }
}

#[test]
fn a_jump_into_a_spot_that_is_not_resident_waits_for_the_ring_only() {
    let out = engine::audio_source::scenario::player_scenario();
    // The third part: the placement entered 12 s in (not resident) while clip 1 played.
    let part: Vec<bool> = out[2 * 350 * BLOCK..2 * 500 * BLOCK]
        .chunks(2)
        .map(|f| f[0] != 0.0 || f[1] != 0.0)
        .collect();
    // Clip 1 fades out, then silence until the ring of the placement delivers.
    let silent = part.iter().position(|s| !s).unwrap();
    let back = silent + part[silent..].iter().position(|s| *s).unwrap();
    let ms = back as f64 * 1000.0 / f64::from(RATE);
    println!(
        "the jump's first frame after {back} frames ({ms:.1} ms; the disk answers one poll late)"
    );
    assert!(silent <= fade_frames(RATE) as usize + 1);
    assert!(back <= 3 * BLOCK, "{back} frames");
}

// ------------------------------------------------- contiguous placements of one clip (#704)

/// A frame placement of `clip` on track 0.
fn part(slot: u32, clip: u32, start: i64, offset: u64, length: u64) -> Placement {
    Placement {
        placement: PlacementSlot(slot),
        clip: ClipSlot(clip),
        start: Tick(start),
        span: Span::Frames {
            offset: Frame(offset),
            length: Frame(length),
        },
    }
}

type Stereo = (Vec<f32>, Vec<f32>);

/// The tempo of the split tests: a tick is not a whole number of samples.
const BPM: f64 = 130.0;

/// A 220 Hz sine of 4 s at `rate` (clip 0, and clip 1 on the same frames) in an engine at
/// `engine` Hz at [`BPM`], with `parts` as track 0's arrangement, played from tick 0 for 600
/// blocks; `mid`: what is sent after 100 blocks.
fn play_parts(engine: u32, rate: u32, parts: Vec<Placement>, mid: &[ControlMsg]) -> Stereo {
    play_parts_at(engine, rate, BPM, parts, mid)
}

/// [`play_parts`] at `bpm`.
fn play_parts_at(
    engine: u32,
    rate: u32,
    bpm: f64,
    parts: Vec<Placement>,
    mid: &[ControlMsg],
) -> Stereo {
    let (mut rig, frames) = sine_rig(engine, rate, 220.0, 4.0, None);
    rig.send(&[
        transport(Quantize::None, bpm),
        ControlMsg::ClipAudioSet {
            clip: ClipSlot(1),
            source: SourceSlot(0),
            start: Frame(0),
            length: Frame(frames),
            gain: 1.0,
            looping: None,
        },
        ControlMsg::TrackPlacementsUpload {
            track: TrackSlot(0),
            first: 0,
            total: parts.len() as u32,
            placements: parts,
        },
        ControlMsg::Play { from: Tick(0) },
    ]);
    let (mut l, mut r) = rig.render(100);
    rig.send(mid);
    let (a, b) = rig.render(500);
    l.extend(a);
    r.extend(b);
    assert_eq!(underruns(&rig), 0);
    (l, r)
}

/// The frames of a source at `rate` before tick `t` at [`BPM`]: where the split command cuts
/// (`floor(t × frames per tick)`).
fn frames_before(rate: u32, t: i64) -> u64 {
    frames_before_at(rate, BPM, t)
}

/// [`frames_before`] at `bpm`.
fn frames_before_at(rate: u32, bpm: f64, t: i64) -> u64 {
    let spt = Tempo::new(bpm)
        .unwrap()
        .samples_per_tick(SampleRate::new(rate).unwrap());
    (t as f64 * spt).floor() as u64
}

/// The block frame where tick `t` plays at [`BPM`] in an engine at `engine` Hz.
fn cut_frame(engine: u32, t: i64) -> usize {
    frames_before(engine, t) as usize
}

/// The largest difference between two renders over `[from, to)`, both channels.
fn max_diff(a: &Stereo, b: &Stereo, from: usize, to: usize) -> f32 {
    (from..to)
        .map(|i| (a.0[i] - b.0[i]).abs().max((a.1[i] - b.1[i]).abs()))
        .fold(0.0, f32::max)
}

fn bit_equal(a: &Stereo, b: &Stereo) {
    assert_eq!(a.0.len(), b.0.len());
    for (i, (x, y)) in a.0.iter().zip(&b.0).chain(a.1.iter().zip(&b.1)).enumerate() {
        assert_eq!(x.to_bits(), y.to_bits(), "sample {i}: {x} and {y}");
    }
}

const CUT: i64 = 1_234;
const CUT2: i64 = 2_345;

#[test]
fn a_placement_split_in_three_plays_like_the_whole_one_sample_for_sample() {
    // Equal rates, a source slower than the engine (less than one source frame per output
    // frame) and one faster (several).
    for (engine, rate) in [(48_000, 48_000), (48_000, 8_000), (11_025, 48_000)] {
        let frames = u64::from(4 * rate);
        let whole = play_parts(engine, rate, vec![part(0, 0, 0, 0, frames)], &[]);
        let (k1, k2) = (frames_before(rate, CUT), frames_before(rate, CUT2));
        let split = play_parts(
            engine,
            rate,
            vec![
                part(0, 0, 0, 0, k1),
                part(1, 0, CUT, k1, k2 - k1),
                part(2, 0, CUT2, k2, frames - k2),
            ],
            &[],
        );
        // The cuts fall where the tone is heard, and the whole render is the same to the
        // bit, the fades included (tolerance 0).
        let at = cut_frame(engine, CUT2);
        assert!(at + 1_000 < whole.0.len());
        assert!(whole.0[at..at + 1_000].iter().any(|s| s.abs() > 0.3));
        bit_equal(&split, &whole);
    }
}

#[test]
fn a_split_plays_on_when_a_tick_is_shorter_than_two_frames() {
    // An 8 kHz file at 400 BPM (1.25 frames a tick) and at 999 BPM (0.5): the left part's
    // end, its frames rounded to ticks, may fall a tick before the cut; that gap, shorter
    // than one more frame, is closed and the right part takes the stream over.
    let (engine, rate) = (48_000, 8_000);
    let frames = u64::from(4 * rate);
    for bpm in [400.0, 999.0] {
        let whole = play_parts_at(engine, rate, bpm, vec![part(0, 0, 0, 0, frames)], &[]);
        let mut gaps = 0;
        for cut in 1_201..1_221 {
            let k = frames_before_at(rate, bpm, cut);
            let own = Tempo::new(bpm).unwrap().audio_end_tick(
                Tick(0),
                Frame(k),
                SampleRate::new(rate).unwrap(),
            );
            let gap = own < Tick(cut);
            gaps += usize::from(gap);
            let split = play_parts_at(
                engine,
                rate,
                bpm,
                vec![part(0, 0, 0, 0, k), part(1, 0, cut, k, frames - k)],
                &[],
            );
            bit_equal(&split, &whole);
            if gap {
                // The same gap before another clip, or before frames that do not follow, is
                // kept: the left part fades out at its own end, and the next one fades in.
                let at = frames_before_at(engine, bpm, own.0) as usize;
                for (clip, offset) in [(1, k), (0, k + 2)] {
                    let other = play_parts_at(
                        engine,
                        rate,
                        bpm,
                        vec![
                            part(0, 0, 0, 0, k),
                            part(1, clip, cut, offset, frames - offset),
                        ],
                        &[],
                    );
                    let first = (0..whole.0.len()).find(|&i| other.0[i] != whole.0[i]);
                    assert_eq!(first, Some(at), "clip {clip} from {offset}: out at the end");
                }
            }
        }
        assert!(gaps > 0, "{bpm} BPM: some cut leaves a rounded gap");
    }
}

#[test]
fn contiguous_placements_whose_frames_do_not_follow_still_crossfade() {
    let (engine, rate) = (48_000, 48_000);
    let frames = u64::from(4 * rate);
    let k = frames_before(rate, CUT);
    let jump = 4_800;
    let whole = play_parts(engine, rate, vec![part(0, 0, 0, 0, frames)], &[]);
    let split = play_parts(
        engine,
        rate,
        vec![
            part(0, 0, 0, 0, k),
            part(1, 0, CUT, k + jump, frames - k - jump),
        ],
        &[],
    );
    let f = cut_frame(engine, CUT);
    let fade = fade_frames(engine) as usize;
    assert_eq!(
        max_diff(&split, &whole, 0, f),
        0.0,
        "the same before the cut"
    );
    // After the fade: the new read, the whole placement's frames `jump` later.
    let j = jump as usize;
    let ahead = (whole.0[j..].to_vec(), whole.1[j..].to_vec());
    let after = max_diff(&split, &ahead, f + fade + 1, whole.0.len() - j);
    assert_eq!(after, 0.0, "the new read after the fade");
    // Within the fade, neither the old read nor the new one: both, crossfaded.
    let old = max_diff(&split, &whole, f + fade / 4, f + 3 * fade / 4);
    let new = max_diff(&split, &ahead, f + fade / 4, f + 3 * fade / 4);
    println!("within the fade: {old} from the old read, {new} from the new one");
    assert!(old > 0.01 && new > 0.01, "a crossfade ({old}, {new})");
    // And no click.
    let source_step = (220.0 * core::f64::consts::TAU / f64::from(rate) * 0.5) as f32;
    let bound = source_step + 1.0 / fade as f32;
    let around = f - fade..f + 2 * fade;
    let step = max_step(&split.0[around.clone()], &split.1[around]);
    assert!(step <= bound, "a click: step {step} > {bound}");
}

#[test]
fn only_the_same_clip_within_the_rounding_of_a_cut_and_without_a_jump_plays_on() {
    let (engine, rate) = (48_000, 48_000);
    let frames = u64::from(4 * rate);
    let k = frames_before(rate, CUT);
    let f = cut_frame(engine, CUT);
    let fade = fade_frames(engine) as usize;
    let whole = play_parts(engine, rate, vec![part(0, 0, 0, 0, frames)], &[]);
    let pair = |clip: u32, offset: u64| {
        vec![
            part(0, 0, 0, 0, k),
            part(1, clip, CUT, offset, frames - offset),
        ]
    };
    // Played from tick 0 at equal rates, the stream is at frame `k` at the cut (`f` is
    // `floor(CUT × frames per tick)` too). Up to `CONTIGUOUS` (two frames) plus one output
    // frame (one frame here) off, either way: the stream plays on at its own position.
    for offset in [k - 3, k - 2, k - 1, k + 1, k + 2, k + 3] {
        bit_equal(&play_parts(engine, rate, pair(0, offset), &[]), &whole);
    }
    // Four frames off, or another clip on the same frames: a crossfade.
    for (clip, offset) in [(0, k + 4), (0, k - 4), (1, k)] {
        let other = play_parts(engine, rate, pair(clip, offset), &[]);
        let d = max_diff(&other, &whole, f, f + fade);
        assert!(d > 0.001, "clip {clip} from {offset}: a crossfade ({d})");
    }
    // A seek into the right part while the left one plays: the transport jumped, the
    // position is computed again under a crossfade, as for the whole placement. The right
    // part's position after the seek is its own, which may differ from the whole
    // placement's by the rounding of its start to a sample (TR-1): one frame.
    let seek = [ControlMsg::Seek { tick: Tick(2_000) }];
    let whole_seek = play_parts(engine, rate, vec![part(0, 0, 0, 0, frames)], &seek);
    let split_seek = play_parts(engine, rate, pair(0, k), &seek);
    let start = 100 * BLOCK;
    assert_eq!(max_diff(&split_seek, &whole_seek, 0, start), 0.0);
    for render in [&whole_seek, &split_seek] {
        let d = max_diff(render, &whole, start + fade / 4, start + 3 * fade / 4);
        assert!(d > 0.01, "the seek crossfades ({d})");
    }
    let shifted = |d: usize| {
        (start + 2 * fade..whole.0.len() - 2).all(|i| {
            split_seek.0[i].to_bits() == whole_seek.0[i + d - 1].to_bits()
                && split_seek.1[i].to_bits() == whole_seek.1[i + d - 1].to_bits()
        })
    };
    assert!(
        shifted(0) || shifted(1) || shifted(2),
        "within a frame of the whole after the seek"
    );
    // A tempo change while the left part plays: the stream keeps its frames (a tempo change
    // does not move a playing clip) while the cut's tick moves, so the right part no longer
    // starts where the stream is and crossfades in (the module doc's limit). The whole
    // placement plays on, without a crossfade.
    let faster = [transport(Quantize::None, 160.0)];
    let whole_faster = play_parts(engine, rate, vec![part(0, 0, 0, 0, frames)], &faster);
    let split_faster = play_parts(engine, rate, pair(0, k), &faster);
    let at = (0..whole_faster.0.len())
        .find(|&i| split_faster.0[i] != whole_faster.0[i])
        .expect("the split crossfades");
    let spt = 60.0 * f64::from(engine) / (160.0 * PPQ as f64);
    let moved = start
        + ((CUT as f64 - start as f64 / (60.0 * f64::from(engine) / (130.0 * PPQ as f64))) * spt)
            .floor() as usize;
    assert!(
        at.abs_diff(moved) <= 2,
        "the crossfade at {at}, the cut at {moved}"
    );
    let d = max_diff(&split_faster, &whole_faster, at, at + fade);
    assert!(d > 0.01, "a crossfade ({d})");
}

#[test]
fn the_parity_scenario_s_split_placements_play_like_the_whole_one() {
    use engine::audio_source::scenario::split_placements;
    for engine in [RATE, 11_025, 6_000] {
        let render = |split: bool| {
            let mut rig = Rig::new(engine, MemDisk::standard(), &player_setup());
            rig.render(200);
            rig.send(&split_placements(split));
            rig.render(100)
        };
        let (whole, split) = (render(false), render(true));
        assert!(whole.0.iter().filter(|s| **s != 0.0).count() > 10_000);
        bit_equal(&split, &whole);
    }
}
