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

/// The track's own fade in (5 ms, `graph::FADE_SECONDS`) and the player's (2 ms), and the
/// gate's: the frames after an onset that may differ from the source.
fn settle_frames(rate: u32) -> usize {
    (rate as usize * 5).div_ceil(1000) + 2 * fade_frames(rate) as usize
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
    // Launched at once: the clip's first frame is the block's first (the track's own fade
    // in, from 0, makes that frame silent).
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
    let steps: [&[ControlMsg]; 7] = [
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
        &[ControlMsg::StopTrack {
            track: TrackSlot(0),
        }],
        &[ControlMsg::Stop],
    ];
    for step in steps {
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
    assert_eq!(frames, 770 * BLOCK);
    // The stop: silent at the end.
    assert!(out[2 * 760 * BLOCK..2 * frames].iter().all(|s| *s == 0.0));
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
