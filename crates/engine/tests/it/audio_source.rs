//! AU-3: audio sources streamed from disk ([`engine::audio_source`]). The deterministic
//! scenario (a disk in memory whose jobs complete one poll late) through the real channel
//! and the host's test voice: a clip start handing over from its preloaded segment to the
//! ring, a jump, a short loop from memory, a short loop through the ring, each compared with
//! the source sample for sample; then the planner's budget, offline sources, stale jobs,
//! backpressure, underruns, and the absence of allocation on the audio side.

use std::sync::atomic::Ordering;

use assert_no_alloc::assert_no_alloc;
use engine::audio_source::scenario::{
    BLOCK, CLIP_START, LONG, LONG_FRAMES, MemDisk, RATE, SHORT, SHORT_FRAMES, long_value, play,
    rig, settle, short_value,
};
use engine::audio_source::{
    DEFAULT_BUDGET_BYTES, FLAG_OVER_BUDGET, Failure, FileKind, MAX_PENDING_FOR_DISK, Path, RING_MS,
};
use engine::host::{Host, Prepare, channel};
use engine::msg::{ClipSlot, ControlMsg, SourceSlot, TrackKind, TrackSlot, encode_stream};
use engine::time::Frame;

fn send(prepare: &mut Prepare, msgs: &[ControlMsg]) {
    let stream = encode_stream(1, 0, msgs).unwrap();
    prepare.submit(&stream).unwrap();
}

fn underruns(host: &Host) -> u32 {
    host.engine().disk_stats().underruns.load(Ordering::Relaxed)
}

/// Checks a run against `expected(frame index) -> [l, r]`: silence while it waited, then
/// every frame equal. Returns the frames waited.
fn check(out: &[f32], waited: usize, expected: impl Fn(usize) -> [f32; 2], what: &str) -> usize {
    let frames = out.len() / 2;
    for i in 0..frames {
        let got = [out[2 * i], out[2 * i + 1]];
        if i < waited {
            assert_eq!(got, [0.0, 0.0], "{what}: frame {i} while waiting");
        } else {
            assert_eq!(got, expected(i), "{what}: frame {i} (waited {waited})");
        }
    }
    waited
}

struct Fx {
    prepare: Prepare,
    host: Host,
    disk: MemDisk,
    clock: u64,
}

fn settled() -> Fx {
    let (mut prepare, mut host) = rig();
    let mut disk = MemDisk::standard();
    let mut clock = 0;
    settle(&mut prepare, &mut host, &mut disk, &mut clock, 200);
    Fx {
        prepare,
        host,
        disk,
        clock,
    }
}

impl Fx {
    /// Renders `blocks` more blocks of the voice already playing.
    fn play_on(&mut self, blocks: usize) -> Vec<f32> {
        play(
            &mut self.prepare,
            &mut self.host,
            &mut self.disk,
            &mut self.clock,
            None,
            blocks,
        )
        .out
    }

    fn play(&mut self, source: u32, path: Path, blocks: usize) -> (Vec<f32>, usize) {
        let run = play(
            &mut self.prepare,
            &mut self.host,
            &mut self.disk,
            &mut self.clock,
            Some((SourceSlot(source), path)),
            blocks,
        );
        (run.out, run.waited)
    }
}

#[test]
fn a_clip_start_plays_from_its_preload_then_the_ring_without_a_gap() {
    let mut f = settled();
    let stats = f.host.engine().disk_stats().clone();
    assert_eq!(
        stats.offline.load(Ordering::Relaxed),
        0,
        "both sources readable"
    );
    // 4 s from the clip's start: 1.5 s preloaded, the rest from the ring.
    let path = Path::new(CLIP_START, CLIP_START + 30 * RATE as u64, None, LONG_FRAMES);
    let (out, waited) = f.play(0, path, 250);
    assert_eq!(waited, 0, "a clip start is resident");
    check(
        &out,
        0,
        |i| {
            let at = CLIP_START + i as u64;
            [long_value(at, 0), long_value(at, 1)]
        },
        "clip start",
    );
    assert!(
        250 * BLOCK > (RATE as usize * 3) / 2,
        "the run outlasts the preload"
    );
    assert_eq!(underruns(&f.host), 0);
}

#[test]
fn a_jump_waits_for_its_ring_then_plays_on_time() {
    let mut f = settled();
    let path = Path::new(200_000, LONG_FRAMES, None, LONG_FRAMES);
    let (out, waited) = f.play(0, path, 200);
    assert!(waited > 0, "200 000 is not resident");
    eprintln!("a jump waited {waited} frames at {RATE} Hz (the disk answers one poll later)");
    assert!(
        waited < RATE as usize / 4,
        "within the grace: {waited} frames"
    );
    // The silence took the place of the frames it skipped: no drift.
    check(
        &out,
        waited,
        |i| {
            let at = 200_000 + i as u64;
            [long_value(at, 0), long_value(at, 1)]
        },
        "jump",
    );
    assert_eq!(underruns(&f.host), 0, "the start's wait is not an underrun");
}

#[test]
fn a_short_loop_plays_from_memory_without_a_gap() {
    let mut f = settled();
    let path = Path::new(0, SHORT_FRAMES, Some((1_000, 1_800)), SHORT_FRAMES);
    let (out, waited) = f.play(1, path, 125);
    assert_eq!(waited, 0);
    check(
        &out,
        0,
        |i| {
            let at = path.advance(0, i as u64);
            [short_value(at, 0), short_value(at, 1)]
        },
        "resident loop",
    );
    assert_eq!(underruns(&f.host), 0);
}

#[test]
fn a_short_loop_through_the_ring_wraps_without_a_gap() {
    let mut f = settled();
    let path = Path::new(299_000, LONG_FRAMES, Some((300_000, 301_600)), LONG_FRAMES);
    let (out, waited) = f.play(0, path, 250);
    assert!(waited > 0);
    check(
        &out,
        waited,
        |i| {
            let at = path.advance(299_000, i as u64);
            [long_value(at, 0), long_value(at, 1)]
        },
        "streamed loop",
    );
    // 250 blocks wrap the 0.2 s loop many times.
    const { assert!(250 * BLOCK > 10 * 1_600) };
    assert_eq!(underruns(&f.host), 0);
}

#[test]
fn the_audio_side_never_allocates_while_voices_stream() {
    let mut f = settled();
    f.host.pump();
    let path = Path::new(150_000, LONG_FRAMES, None, LONG_FRAMES);
    assert_no_alloc(|| {
        assert!(f.host.probe_start(TrackSlot(0), 1, SourceSlot(0), path));
    });
    for _ in 0..300 {
        assert_no_alloc(|| {
            f.host.pump();
            f.host.probe_read(BLOCK);
            f.host.write_feedback();
        });
        f.clock += BLOCK as u64;
        f.disk
            .step(&mut f.prepare, f.clock * 1000 / u64::from(RATE));
        f.prepare.flush();
    }
    assert!(f.host.probe().position() > 150_000, "it streamed");
}

#[test]
fn a_stalled_disk_is_counted_as_underruns_and_plays_silence() {
    let mut f = settled();
    let path = Path::new(100_000, LONG_FRAMES, None, LONG_FRAMES);
    let (_, waited) = f.play(0, path, 50);
    assert!(waited > 0);
    assert_eq!(underruns(&f.host), 0);
    // The disk stops answering: the ring runs dry (2.5 s of lookahead), then each block is
    // an underrun, and the voice still advances.
    let before = f.host.probe().position();
    for _ in 0..400 {
        f.host.pump();
        f.host.probe_read(BLOCK);
        f.prepare.flush();
    }
    assert!(underruns(&f.host) > 0, "the dry ring counts");
    assert_eq!(f.host.probe().position(), before + 400 * BLOCK as u64);
    // A request whose ring never gets data (the disk stopped answering) counts too, once
    // past the grace.
    let mut g = settled();
    g.disk.stall();
    g.host.pump();
    assert!(g.host.probe_start(
        TrackSlot(0),
        0,
        SourceSlot(0),
        Path::new(250_000, LONG_FRAMES, None, LONG_FRAMES)
    ));
    for _ in 0..40 {
        g.host.pump();
        g.host.probe_read(BLOCK);
        g.clock += BLOCK as u64;
        g.disk
            .step(&mut g.prepare, g.clock * 1000 / u64::from(RATE));
        g.prepare.flush();
    }
    assert!(
        underruns(&g.host) > 0,
        "a stalled start counts after the grace"
    );
}

#[test]
fn a_missing_blob_is_an_offline_source_until_it_comes_back() {
    let (mut prepare, mut host) = rig();
    let mut disk = MemDisk::standard();
    let long = disk.files.remove(&(LONG, FileKind::Blob as u8)).unwrap();
    let mut clock = 0;
    settle(&mut prepare, &mut host, &mut disk, &mut clock, 20);
    host.write_feedback();
    let fb = engine::host::read_feedback(host.feedback()).unwrap();
    assert_eq!(fb.offline_sources, 1, "the long source's blob is missing");
    // A voice on it is silent and asks for nothing it could get.
    let path = Path::new(CLIP_START, LONG_FRAMES, None, LONG_FRAMES);
    let run = play(
        &mut prepare,
        &mut host,
        &mut disk,
        &mut clock,
        Some((SourceSlot(0), path)),
        10,
    );
    assert!(run.out.iter().all(|s| *s == 0.0));
    // The blob comes back: probed again within REPROBE_MS.
    disk.files.insert((LONG, FileKind::Blob as u8), long);
    settle(&mut prepare, &mut host, &mut disk, &mut clock, 200);
    host.write_feedback();
    let fb = engine::host::read_feedback(host.feedback()).unwrap();
    assert_eq!(fb.offline_sources, 0);
    assert!(fb.pcm_kib > 0);
}

#[test]
fn an_engine_without_a_disk_reports_every_source_offline() {
    // The helper since BO-1: nobody polls the planner.
    let (mut prepare, mut host) = rig();
    host.pump();
    host.write_feedback();
    let fb = engine::host::read_feedback(host.feedback()).unwrap();
    assert_eq!(fb.offline_sources, 2);
    assert_eq!(fb.pcm_kib, 0, "nothing loaded, no ring");
    prepare.flush();
}

#[test]
fn the_budget_holds_when_the_sources_ask_for_more() {
    let (mut prepare, mut host) = channel(RATE as f32).unwrap();
    let budget = 2_000_000;
    prepare.disk_mut().set_budget(budget);
    let mut msgs = vec![
        ControlMsg::Reset { epoch: 1 },
        ControlMsg::TrackCreate {
            track: TrackSlot(0),
            kind: TrackKind::Audio,
        },
    ];
    let mut disk = MemDisk::default();
    // Twenty short sources of 10 s stereo (640 KB each decoded): 6.4 times the budget.
    for i in 0..20u8 {
        let media = [i + 1; 32];
        msgs.push(ControlMsg::AudioSourceDeclare {
            source: SourceSlot(u32::from(i)),
            sample_rate: RATE,
            channels: 2,
            frames: Frame(SHORT_FRAMES),
            media,
        });
        disk.files
            .insert((media, FileKind::Blob as u8), b"x".to_vec());
        disk.files.insert(
            (media, FileKind::Cache as u8),
            engine::audio_source::scenario::short_cache(),
        );
    }
    send(&mut prepare, &msgs);
    let mut clock = 0;
    for _ in 0..2_000 {
        host.pump();
        clock += BLOCK as u64;
        disk.step(&mut prepare, clock * 1000 / u64::from(RATE));
        prepare.flush();
        assert!(
            prepare.disk().pcm_bytes() <= budget,
            "{} > {budget}",
            prepare.disk().pcm_bytes()
        );
    }
    assert!(
        prepare.disk().pcm_bytes() > budget / 2,
        "it fills what it may"
    );
    // A voice on source 19 is never cut by the eviction while it plays (in use: kept).
    let path = Path::new(0, SHORT_FRAMES, Some((0, SHORT_FRAMES)), SHORT_FRAMES);
    let run = play(
        &mut prepare,
        &mut host,
        &mut disk,
        &mut clock,
        Some((SourceSlot(19), path)),
        1_000,
    );
    let tail = &run.out[run.out.len() - 2 * BLOCK * 100..];
    assert!(tail.iter().any(|s| *s != 0.0), "still playing after 16 s");
    assert_eq!(underruns(&host), 0);
    assert!(prepare.disk().pcm_bytes() <= budget);
    assert_eq!(DEFAULT_BUDGET_BYTES, 300_000_000);
}

#[test]
fn rings_over_the_budget_are_kept_and_flagged_never_cut() {
    // § 5.4: the buffers of the voices that play are never evicted; over the budget, the
    // report says so instead of cutting the sound.
    let (mut prepare, mut host) = rig();
    let ring = u64::from(RATE) * RING_MS / 1000 * 2 * 4;
    prepare.disk_mut().set_budget(ring + ring / 2);
    let mut disk = MemDisk::standard();
    let mut clock = 0;
    settle(&mut prepare, &mut host, &mut disk, &mut clock, 50);
    let path = |at| Path::new(at, LONG_FRAMES, None, LONG_FRAMES);
    host.pump();
    assert!(host.probe_start(TrackSlot(0), 0, SourceSlot(0), path(100_000)));
    settle(&mut prepare, &mut host, &mut disk, &mut clock, 5);
    assert!(host.probe_start(TrackSlot(0), 1, SourceSlot(0), path(200_000)));
    settle(&mut prepare, &mut host, &mut disk, &mut clock, 5);
    let t = host.engine().state().track(TrackSlot(0)).unwrap();
    assert!(
        t.ports.iter().all(|p| p.ring.is_some()),
        "both voices have their ring"
    );
    assert!(prepare.disk().pcm_bytes() > prepare.disk().budget());
    host.write_feedback();
    let fb = engine::host::read_feedback(host.feedback()).unwrap();
    assert_eq!(fb.pcm_flags & FLAG_OVER_BUDGET, FLAG_OVER_BUDGET);
}

#[test]
fn a_ring_left_dry_for_long_comes_back_with_its_source() {
    let mut f = settled();
    let path = Path::new(100_000, LONG_FRAMES, None, LONG_FRAMES);
    let (_, waited) = f.play(0, path, 20);
    assert!(waited > 0);
    // The blob goes away for 15 s (past DETACH_MS) while the voice plays on.
    let wav = f.disk.files.remove(&(LONG, FileKind::Blob as u8)).unwrap();
    let blocks = 15 * RATE as usize / BLOCK;
    let run = f.play_on(blocks);
    assert!(run.iter().rev().take(BLOCK * 2).all(|s| *s == 0.0));
    f.disk.files.insert((LONG, FileKind::Blob as u8), wav);
    let tail = f.play_on(5 * RATE as usize / BLOCK);
    let at = f.host.probe().position();
    assert!(
        tail[tail.len() - 2..] != [0.0, 0.0],
        "playing again at {at}"
    );
    let n = tail.len() / 2;
    assert_eq!(tail[2 * (n - 1)], long_value(at - 1, 0), "on time");
}

#[test]
fn a_port_reused_for_a_source_of_another_shape_gets_a_ring_of_that_shape() {
    let mut f = settled();
    let (_, _) = f.play(0, Path::new(100_000, LONG_FRAMES, None, LONG_FRAMES), 20);
    // A mono source whose cache appears only later: the stereo ring must not stay.
    let media = [0xd4; 32];
    send(
        &mut f.prepare,
        &[ControlMsg::AudioSourceDeclare {
            source: SourceSlot(2),
            sample_rate: RATE,
            channels: 1,
            frames: Frame(LONG_FRAMES),
            media,
        }],
    );
    let mut cache = vec![0u8; 32];
    cache[..8].copy_from_slice(engine::audio_source::CACHE_MAGIC);
    cache[8..12].copy_from_slice(&RATE.to_le_bytes());
    cache[12..14].copy_from_slice(&1u16.to_le_bytes());
    cache[16..24].copy_from_slice(&LONG_FRAMES.to_le_bytes());
    for i in 0..LONG_FRAMES {
        cache.extend_from_slice(&((i as f32) / 1e6 + 0.25).to_le_bytes());
    }
    f.disk
        .files
        .insert((media, FileKind::Blob as u8), b"x".to_vec());
    f.host.pump();
    assert!(f.host.probe_start(
        TrackSlot(0),
        0,
        SourceSlot(2),
        Path::new(200_000, LONG_FRAMES, None, LONG_FRAMES)
    ));
    settle(&mut f.prepare, &mut f.host, &mut f.disk, &mut f.clock, 10);
    f.disk.files.insert((media, FileKind::Cache as u8), cache);
    let run = f.play_on(5 * RATE as usize / BLOCK);
    assert!(run.iter().any(|s| *s > 0.2), "the mono source plays");
    let t = f.host.engine().state().track(TrackSlot(0)).unwrap();
    assert_eq!(t.ports[0].ring.as_deref().unwrap().channels, 1);
}

#[test]
fn a_file_s_nan_and_subnormal_samples_never_reach_the_voice() {
    let (mut prepare, mut host) = rig();
    let mut disk = MemDisk::standard();
    let cache = disk.files.get_mut(&(SHORT, FileKind::Cache as u8)).unwrap();
    for (k, v) in [f32::NAN, f32::INFINITY, 1e-40].iter().enumerate() {
        let at = 32 + k * 4;
        cache[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }
    let mut clock = 0;
    settle(&mut prepare, &mut host, &mut disk, &mut clock, 200);
    let run = play(
        &mut prepare,
        &mut host,
        &mut disk,
        &mut clock,
        Some((
            SourceSlot(1),
            Path::new(0, SHORT_FRAMES, None, SHORT_FRAMES),
        )),
        4,
    );
    assert!(
        run.out
            .iter()
            .all(|s| s.is_finite() && (*s == 0.0 || s.is_normal()))
    );
    assert_eq!(&run.out[..3], &[0.0, 0.0, 0.0]);
}

#[test]
fn a_ring_is_sized_for_its_source_rate_and_channels() {
    // 96 kHz, 6 channels: the ring holds 3 s of them, so the 2.5 s lookahead holds too.
    let rate = 96_000;
    let (mut prepare, mut host) = channel(48_000.0).unwrap();
    let media = [7; 32];
    let frames = 60 * u64::from(rate);
    send(
        &mut prepare,
        &[
            ControlMsg::Reset { epoch: 1 },
            ControlMsg::TrackCreate {
                track: TrackSlot(0),
                kind: TrackKind::Audio,
            },
            ControlMsg::AudioSourceDeclare {
                source: SourceSlot(0),
                sample_rate: rate,
                channels: 6,
                frames: Frame(frames),
                media,
            },
        ],
    );
    // A cache header that matches, and a file of the right size made of zeros.
    let mut cache = vec![0u8; 32];
    cache[..8].copy_from_slice(engine::audio_source::CACHE_MAGIC);
    cache[8..12].copy_from_slice(&rate.to_le_bytes());
    cache[12..14].copy_from_slice(&6u16.to_le_bytes());
    cache[16..24].copy_from_slice(&frames.to_le_bytes());
    cache.resize(32 + frames as usize * 6 * 4, 0);
    let mut disk = MemDisk::default();
    disk.files
        .insert((media, FileKind::Blob as u8), b"x".to_vec());
    disk.files.insert((media, FileKind::Cache as u8), cache);
    let mut clock = 0;
    settle(&mut prepare, &mut host, &mut disk, &mut clock, 10);
    host.pump();
    assert!(host.probe_start(
        TrackSlot(0),
        0,
        SourceSlot(0),
        Path::new(1_000_000, frames, None, frames)
    ));
    settle(&mut prepare, &mut host, &mut disk, &mut clock, 400);
    let state = host.engine().state();
    let ring = state.track(TrackSlot(0)).unwrap().ports[0]
        .ring
        .as_deref()
        .unwrap();
    assert_eq!(ring.channels, 6, "every channel kept");
    assert_eq!(ring.frames, u64::from(rate) * RING_MS / 1000);
    let fill = ring.write.load(Ordering::Relaxed) - ring.read.load(Ordering::Relaxed);
    assert!(
        fill >= u64::from(rate) * 2,
        "at least 2 s ahead: {fill} frames"
    );
}

#[test]
fn a_job_of_a_released_source_never_lands_in_the_source_that_takes_its_slot() {
    let (mut prepare, mut host) = rig();
    let mut disk = MemDisk::standard();
    let mut clock = 0;
    // Probes and the first segment jobs go out...
    settle(&mut prepare, &mut host, &mut disk, &mut clock, 3);
    // ...then slot 1 is released and given to another media whose samples are all 0.5.
    let other = [0xc3; 32];
    let mut cache = engine::audio_source::scenario::short_cache();
    for f in cache[32..].as_chunks_mut::<4>().0 {
        *f = 0.5f32.to_le_bytes();
    }
    disk.files
        .insert((other, FileKind::Blob as u8), b"x".to_vec());
    disk.files.insert((other, FileKind::Cache as u8), cache);
    send(
        &mut prepare,
        &[
            ControlMsg::AudioSourceRelease {
                source: SourceSlot(1),
            },
            ControlMsg::AudioSourceDeclare {
                source: SourceSlot(1),
                sample_rate: RATE,
                channels: 2,
                frames: Frame(SHORT_FRAMES),
                media: other,
            },
        ],
    );
    settle(&mut prepare, &mut host, &mut disk, &mut clock, 300);
    let src = host.engine().state().source(SourceSlot(1)).unwrap();
    let segs: Vec<_> = src.segments.iter().flatten().collect();
    assert!(!segs.is_empty(), "the new media is loaded");
    for s in segs {
        assert!(
            s.data.iter().all(|v| *v == 0.5),
            "only the new media's samples"
        );
    }
    let _ = SHORT;
}

#[test]
fn a_cache_that_changed_is_probed_again() {
    let mut f = settled();
    // Serve one job of the short source as "changed": it goes back to probing, then online.
    let job = f
        .prepare
        .disk_poll(f.clock * 1000 / u64::from(RATE) + 10_000)
        .first()
        .copied();
    if let Some(job) = job {
        f.prepare.disk_failed(job.id, Failure::Changed);
    }
    settle(&mut f.prepare, &mut f.host, &mut f.disk, &mut f.clock, 300);
    f.host.write_feedback();
    let fb = engine::host::read_feedback(f.host.feedback()).unwrap();
    assert_eq!(fb.offline_sources, 0);
}

#[test]
fn the_planner_leaves_room_in_the_queue_for_the_project() {
    // The engine consumes nothing (no pump): the planner's messages stop at its share of
    // the queue, and a project message is still taken.
    let (mut prepare, _host) = channel(RATE as f32).unwrap();
    let mut msgs = vec![ControlMsg::Reset { epoch: 1 }];
    let mut disk = MemDisk::default();
    for i in 0..200u32 {
        let media = [
            (i % 250) as u8 + 1,
            (i / 250) as u8,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
        ];
        msgs.push(ControlMsg::AudioSourceDeclare {
            source: SourceSlot(i),
            sample_rate: RATE,
            channels: 1,
            frames: Frame(800),
            media,
        });
        disk.files
            .insert((media, FileKind::Blob as u8), b"x".to_vec());
        let mut cache = vec![0u8; 32];
        cache[..8].copy_from_slice(engine::audio_source::CACHE_MAGIC);
        cache[8..12].copy_from_slice(&RATE.to_le_bytes());
        cache[12..14].copy_from_slice(&1u16.to_le_bytes());
        cache[16..24].copy_from_slice(&800u64.to_le_bytes());
        cache.resize(32 + 800 * 4, 0);
        disk.files.insert((media, FileKind::Cache as u8), cache);
    }
    // The control stream itself fills the rt ring past what the engine takes.
    let stream = encode_stream(1, 0, &msgs).unwrap();
    prepare.submit(&stream).unwrap();
    for t in 0..3_000u64 {
        disk.step(&mut prepare, t * 10);
    }
    assert!(
        prepare.preparer().pending() <= MAX_PENDING_FOR_DISK + 1,
        "{} pending",
        prepare.preparer().pending()
    );
    send(
        &mut prepare,
        &[ControlMsg::TrackCreate {
            track: TrackSlot(0),
            kind: TrackKind::Midi,
        }],
    );
}

#[test]
fn a_track_replaced_in_its_slot_never_gets_the_old_tracks_ring() {
    let mut f = settled();
    f.host.pump();
    let path = Path::new(100_000, LONG_FRAMES, None, LONG_FRAMES);
    assert!(f.host.probe_start(TrackSlot(0), 0, SourceSlot(0), path));
    // The request reaches the planner, which queues the ring; the slot changes hands first.
    let now = f.clock * 1000 / u64::from(RATE);
    f.prepare.disk_poll(now);
    send(
        &mut f.prepare,
        &[
            ControlMsg::TrackRemove {
                track: TrackSlot(0),
            },
            ControlMsg::TrackCreate {
                track: TrackSlot(0),
                kind: TrackKind::Audio,
            },
        ],
    );
    settle(&mut f.prepare, &mut f.host, &mut f.disk, &mut f.clock, 20);
    let t = f.host.engine().state().track(TrackSlot(0)).unwrap();
    assert!(
        t.ports.iter().all(|p| p.ring.is_none()),
        "the new track asked for nothing"
    );
    let _ = ClipSlot(0);
}
