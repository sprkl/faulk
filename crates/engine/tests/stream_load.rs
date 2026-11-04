//! A measurement, not a check (AU-3): underruns of disk streaming under load, natively, with
//! the audio side and the prepare side on two threads as the helper runs them. 48 voices on 24
//! audio tracks stream 48 kHz stereo 16-bit WAVs (60 s, so nothing is kept whole in memory)
//! from positions no segment covers; the "disk" answers each read after a random 0 to
//! `--latency` ms; the audio thread renders 128-frame blocks in real time. Prints the
//! underruns, the PCM in memory and the frames each voice waited for its first data.
//!
//! `cargo test -p engine --release --test stream_load -- --ignored --nocapture`
//! (`FAULK_LOAD_SECONDS`, `FAULK_LOAD_LATENCY_MS` and `FAULK_LOAD_BURN` change the run).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use engine::audio_source::{FileKind, Path, PortId, Voice};
use engine::host::channel;
use engine::msg::{ControlMsg, SourceSlot, TrackKind, TrackSlot, encode_stream};
use engine::time::Frame;

const RATE: u32 = 48_000;
const BLOCK: usize = 128;
const SOURCES: usize = 6;
const TRACKS: usize = 24;
const SECONDS_OF_SOURCE: u64 = 60;

fn env(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn wav(seed: u64) -> Vec<u8> {
    let frames = SECONDS_OF_SOURCE * u64::from(RATE);
    let data = frames as usize * 4;
    let mut w = Vec::with_capacity(44 + data);
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data as u32).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&RATE.to_le_bytes());
    w.extend_from_slice(&(RATE * 4).to_le_bytes());
    w.extend_from_slice(&4u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&(data as u32).to_le_bytes());
    for f in 0..frames {
        let v = (((f + seed) % 20_000) as i32 - 10_000) as i16;
        w.extend_from_slice(&v.to_le_bytes());
        w.extend_from_slice(&v.to_le_bytes());
    }
    w
}

/// A tiny deterministic generator for the latencies.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self, max: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        if max == 0 {
            0
        } else {
            (self.0 >> 33) % (max + 1)
        }
    }
}

#[test]
#[ignore = "a measurement: run it with --ignored --nocapture"]
fn underruns_under_load() {
    let seconds = env("FAULK_LOAD_SECONDS", 10);
    let latency = env("FAULK_LOAD_LATENCY_MS", 20);
    let burn = env("FAULK_LOAD_BURN", 0) != 0;
    let (mut prepare, mut host) = channel(RATE as f32).unwrap();
    let frames = SECONDS_OF_SOURCE * u64::from(RATE);
    let mut msgs = vec![ControlMsg::Reset { epoch: 1 }];
    for t in 0..TRACKS as u32 {
        msgs.push(ControlMsg::TrackCreate {
            track: TrackSlot(t),
            kind: TrackKind::Audio,
        });
    }
    let mut files = std::collections::HashMap::new();
    for s in 0..SOURCES {
        let media = [s as u8 + 1; 32];
        msgs.push(ControlMsg::AudioSourceDeclare {
            source: SourceSlot(s as u32),
            sample_rate: RATE,
            channels: 2,
            frames: Frame(frames),
            media,
        });
        files.insert(media, Arc::new(wav(s as u64 * 977)));
    }
    prepare
        .submit(&encode_stream(1, 0, &msgs).unwrap())
        .unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let start = Instant::now();

    // The prepare side and its disk: jobs answered after a random delay.
    let disk_stop = Arc::clone(&stop);
    let disk = std::thread::spawn(move || {
        let mut rng = Lcg(7);
        let mut waiting: Vec<(Instant, engine::audio_source::Job)> = Vec::new();
        let mut served = 0u64;
        while !disk_stop.load(Ordering::Relaxed) {
            let now = Instant::now();
            let (due, later): (Vec<_>, Vec<_>) = waiting.into_iter().partition(|(t, _)| *t <= now);
            waiting = later;
            for (_, job) in due {
                let file = &files[&job.media];
                assert_eq!(job.file, FileKind::Blob);
                let from = (job.offset as usize).min(file.len());
                let to = (from + job.len as usize).min(file.len());
                prepare.disk_done(job.id, &file[from..to], file.len() as u64);
                served += 1;
            }
            let ms = start.elapsed().as_millis() as u64;
            let polled = prepare.disk_poll(ms).to_vec();
            if std::env::var_os("FAULK_LOAD_DEBUG").is_some() && !polled.is_empty() {
                eprintln!(
                    "t={ms} jobs={} inflight={} lens={:?}",
                    polled.len(),
                    prepare.disk().in_flight(),
                    polled.iter().map(|j| j.len).collect::<Vec<_>>()
                );
            }
            for job in polled {
                waiting.push((now + Duration::from_millis(rng.next(latency)), job));
            }
            prepare.flush();
            std::thread::sleep(Duration::from_millis(1));
        }
        (served, prepare.disk().pcm_bytes())
    });

    // The audio side: 48 voices, real-time blocks.
    let mut voices = [Voice::default(); 2 * TRACKS];
    let mut out = vec![0.0f32; BLOCK * 2];
    let mut waited = [0usize; 2 * TRACKS];
    let mut started = [false; 2 * TRACKS];
    let block = Duration::from_secs_f64(BLOCK as f64 / f64::from(RATE));
    // Let the probes finish.
    std::thread::sleep(Duration::from_millis(300));
    host.pump();
    {
        let (state, tx, _) = host.engine_mut().streams();
        for (v, voice) in voices.iter_mut().enumerate() {
            let (t, p) = (v / 2, v % 2);
            let s = v % SOURCES;
            let src = state.sources.as_slice()[s].as_deref().unwrap();
            let track = state.tracks.as_mut_slice()[t].as_deref_mut().unwrap();
            let instance = track.instance;
            // Far from any clip start: nothing resident, everything through the ring.
            let at = 5 * u64::from(RATE) + v as u64 * 17_000;
            voice.start(
                PortId {
                    track: TrackSlot(t as u32),
                    instance,
                    port: p as u8,
                },
                &mut track.ports[p],
                SourceSlot(s as u32),
                RATE,
                2,
                &src.segments,
                Path::new(at, frames, None, frames),
                tx,
            );
        }
    }
    let mut next = Instant::now();
    let blocks = (seconds * u64::from(RATE)) as usize / BLOCK;
    let mut late = 0u64;
    let mut worst = Duration::ZERO;
    for _ in 0..blocks {
        let began = Instant::now();
        host.pump();
        let (state, tx, stats) = host.engine_mut().streams();
        for (v, voice) in voices.iter_mut().enumerate() {
            let (t, p) = (v / 2, v % 2);
            let src = state.sources.as_slice()[v % SOURCES].as_deref().unwrap();
            let track = state.tracks.as_mut_slice()[t].as_deref_mut().unwrap();
            let got = voice.read(
                &mut track.ports[p],
                &src.segments,
                tx,
                stats,
                &mut out,
                BLOCK,
            );
            if !started[v] {
                if got == 0 {
                    waited[v] += BLOCK;
                } else {
                    waited[v] += BLOCK - got;
                    started[v] = true;
                }
            }
        }
        if burn {
            // A render that takes about half the block.
            let until = began + block / 2;
            while Instant::now() < until {
                std::hint::spin_loop();
            }
        }
        worst = worst.max(began.elapsed());
        next += block;
        let now = Instant::now();
        if now > next {
            late += 1;
        } else {
            while Instant::now() < next {
                std::thread::yield_now();
            }
        }
    }
    let underruns = host.engine().disk_stats().underruns.load(Ordering::Relaxed);
    stop.store(true, Ordering::Relaxed);
    let (served, pcm) = disk.join().unwrap();
    let max_wait = waited.iter().max().copied().unwrap_or(0);
    let mean_wait = waited.iter().sum::<usize>() as f64 / waited.len() as f64;
    println!(
        "stream_load: {seconds} s, {} voices, disk latency 0..{latency} ms, burn {burn}: \
         underruns {underruns} (of {} voice-blocks), late blocks {late}, worst block {:?}, \
         reads {served}, PCM in memory {:.1} MB, first data after {:.1} ms on average, {:.1} ms at most",
        voices.len(),
        blocks * voices.len(),
        worst,
        pcm as f64 / 1e6,
        mean_wait * 1000.0 / f64::from(RATE),
        max_wait as f64 * 1000.0 / f64::from(RATE),
    );
    assert!(started.iter().all(|s| *s), "every voice played");
}
