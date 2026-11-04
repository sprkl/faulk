//! A deterministic streaming scenario (AU-3): the real channel ([`crate::host::channel`]), a
//! disk held in memory whose jobs complete one poll late, and the host's test voice, through
//! a clip start (preloaded segment handing over to the ring), a jump into a spot that is not
//! resident, a short loop played from memory, and a short loop streamed through the ring.
//! The engine tests compare its output with the source sample for sample; the native/WASM
//! parity test (`crates/engine-wasm/tests/stream_parity.rs`) compares the two builds.

use std::collections::HashMap;

use super::{FileKind, Job, Path};
use crate::host::{Host, Prepare, channel};
use crate::msg::{ClipSlot, ControlMsg, SourceSlot, TrackKind, TrackSlot, encode_stream};
use crate::time::Frame;

/// The engine's rate and the sources'.
pub const RATE: u32 = 8_000;
/// Frames per block.
pub const BLOCK: usize = 128;
/// The long source: a 16-bit stereo WAV read directly, 40 s.
pub const LONG_FRAMES: u64 = 40 * RATE as u64;
/// The short source: a PCM cache (its blob is not a direct format), 10 s, stereo.
pub const SHORT_FRAMES: u64 = 10 * RATE as u64;
/// Where clip 0 starts in the long source.
pub const CLIP_START: u64 = 2 * RATE as u64;

pub const LONG: [u8; 32] = [0xa1; 32];
pub const SHORT: [u8; 32] = [0xb2; 32];

/// Sample `c` of frame `f` of the long source, as its WAV stores it.
pub fn long_sample(f: u64, c: usize) -> i16 {
    (((f.wrapping_mul(7).wrapping_add(c as u64 * 3)) % 60_000) as i32 - 30_000) as i16
}

/// The long source's sample as the engine reads it.
pub fn long_value(f: u64, c: usize) -> f32 {
    f32::from(long_sample(f, c)) / 32_768.0
}

/// The short source's sample.
pub fn short_value(f: u64, c: usize) -> f32 {
    f as f32 / 1_000_000.0 + c as f32
}

/// A WAV of the long source.
pub fn long_wav() -> Vec<u8> {
    let data = LONG_FRAMES as usize * 4;
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
    for f in 0..LONG_FRAMES {
        for c in 0..2 {
            w.extend_from_slice(&long_sample(f, c).to_le_bytes());
        }
    }
    w
}

/// The PCM cache of the short source.
pub fn short_cache() -> Vec<u8> {
    let mut w = Vec::with_capacity(32 + SHORT_FRAMES as usize * 8);
    w.extend_from_slice(super::CACHE_MAGIC);
    w.extend_from_slice(&RATE.to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&0u16.to_le_bytes());
    w.extend_from_slice(&SHORT_FRAMES.to_le_bytes());
    w.extend_from_slice(&1u32.to_le_bytes());
    w.extend_from_slice(&0u32.to_le_bytes());
    for f in 0..SHORT_FRAMES {
        for c in 0..2 {
            w.extend_from_slice(&short_value(f, c).to_le_bytes());
        }
    }
    w
}

/// Files by media and kind.
#[derive(Default)]
pub struct MemDisk {
    pub files: HashMap<([u8; 32], u8), Vec<u8>>,
    /// Jobs taken at the last poll, completed at the next.
    waiting: Vec<Job>,
    stalled: bool,
}

impl MemDisk {
    /// The scenario's two media: the long WAV as a blob, the short source as a blob that is
    /// not a direct format plus its cache.
    pub fn standard() -> MemDisk {
        let mut d = MemDisk::default();
        d.files.insert((LONG, FileKind::Blob as u8), long_wav());
        d.files
            .insert((SHORT, FileKind::Blob as u8), b"not a wav".to_vec());
        d.files
            .insert((SHORT, FileKind::Cache as u8), short_cache());
        d
    }

    /// From now on, never answers: its jobs stay in flight (a disk that hangs).
    pub fn stall(&mut self) {
        self.stalled = true;
    }

    /// Answers again: the jobs still in flight complete at the next step.
    pub fn unstall(&mut self) {
        self.stalled = false;
    }

    /// Completes the jobs of the previous poll, then polls at `now_ms` and keeps its jobs.
    pub fn step(&mut self, prepare: &mut Prepare, now_ms: u64) {
        if !self.stalled {
            for job in core::mem::take(&mut self.waiting) {
                self.serve(prepare, &job);
            }
        }
        self.waiting = prepare.disk_poll(now_ms).to_vec();
    }

    /// Serves one job.
    pub fn serve(&self, prepare: &mut Prepare, job: &Job) {
        match self.files.get(&(job.media, job.file as u8)) {
            Some(file) => {
                let from = (job.offset as usize).min(file.len());
                let to = from.saturating_add(job.len as usize).min(file.len());
                prepare.disk_done(job.id, &file[from..to], file.len() as u64);
            }
            None => prepare.disk_failed(job.id, super::Failure::NotFound),
        }
    }
}

/// The scenario's channel: an audio track in slot 0, the two sources, clip 0 on the long one
/// from [`CLIP_START`], clip 1 the short one with a loop.
pub fn rig() -> (Prepare, Host) {
    let (mut prepare, host) = channel(RATE as f32).expect("a channel");
    prepare.submit(&rig_stream()).expect("accepted");
    (prepare, host)
}

/// The control stream of [`rig`], epoch 1 (the web test replays it: `stream.stream`).
pub fn rig_stream() -> Vec<u8> {
    let msgs = [
        ControlMsg::Reset { epoch: 1 },
        ControlMsg::TrackCreate {
            track: TrackSlot(0),
            kind: TrackKind::Audio,
        },
        ControlMsg::AudioSourceDeclare {
            source: SourceSlot(0),
            sample_rate: RATE,
            channels: 2,
            frames: Frame(LONG_FRAMES),
            media: LONG,
        },
        ControlMsg::AudioSourceDeclare {
            source: SourceSlot(1),
            sample_rate: RATE,
            channels: 2,
            frames: Frame(SHORT_FRAMES),
            media: SHORT,
        },
        ControlMsg::ClipAudioSet {
            clip: ClipSlot(0),
            source: SourceSlot(0),
            start: Frame(CLIP_START),
            length: Frame(30 * RATE as u64),
            gain: 1.0,
            looping: None,
        },
        ControlMsg::ClipAudioSet {
            clip: ClipSlot(1),
            source: SourceSlot(1),
            start: Frame(0),
            length: Frame(SHORT_FRAMES),
            gain: 1.0,
            looping: Some((Frame(1_000), Frame(1_800))),
        },
    ];
    encode_stream(1, 0, &msgs).expect("encodes")
}

/// One run: blocks of the test voice, the disk stepped each block (its clock is the rendered
/// time). Returns the interleaved stereo output and the frames waited before the first
/// frame.
pub struct Run {
    pub out: Vec<f32>,
    pub waited: usize,
}

/// Renders `blocks` blocks of the test voice after `start`, the disk stepped every block.
pub fn play(
    prepare: &mut Prepare,
    host: &mut Host,
    disk: &mut MemDisk,
    clock: &mut u64,
    start: Option<(SourceSlot, Path)>,
    blocks: usize,
) -> Run {
    host.pump();
    if let Some((source, path)) = start {
        assert!(host.probe_start(TrackSlot(0), 0, source, path));
    }
    let mut out = Vec::with_capacity(blocks * BLOCK * 2);
    let mut waited = 0usize;
    let mut started = false;
    for _ in 0..blocks {
        host.pump();
        let got = host.probe_read(BLOCK);
        if !started {
            if got == 0 {
                waited += BLOCK;
            } else {
                waited += BLOCK - got;
                started = true;
            }
        }
        out.extend_from_slice(&host.probe_out()[..BLOCK * 2]);
        *clock += BLOCK as u64;
        disk.step(prepare, *clock * 1000 / u64::from(RATE));
        prepare.flush();
    }
    Run { out, waited }
}

/// Lets the disk settle (probes, segments) for `blocks` blocks without a voice.
pub fn settle(
    prepare: &mut Prepare,
    host: &mut Host,
    disk: &mut MemDisk,
    clock: &mut u64,
    blocks: usize,
) {
    for _ in 0..blocks {
        host.pump();
        *clock += BLOCK as u64;
        disk.step(prepare, *clock * 1000 / u64::from(RATE));
        prepare.flush();
    }
    host.pump();
}

/// The whole scenario's output, for the parity test: the four runs one after the other,
/// then the frames each waited and the underruns.
pub fn scenario() -> Vec<f32> {
    let (mut prepare, mut host) = rig();
    let mut disk = MemDisk::standard();
    let mut clock = 0;
    settle(&mut prepare, &mut host, &mut disk, &mut clock, 200);
    let long = |start: u64, end: u64, looping| Path::new(start, end, looping, LONG_FRAMES);
    let runs = [
        (
            SourceSlot(0),
            long(CLIP_START, CLIP_START + 30 * RATE as u64, None),
            250,
        ),
        (SourceSlot(0), long(200_000, LONG_FRAMES, None), 200),
        (
            SourceSlot(1),
            Path::new(0, SHORT_FRAMES, Some((1_000, 1_800)), SHORT_FRAMES),
            125,
        ),
        (
            SourceSlot(0),
            long(299_000, LONG_FRAMES, Some((300_000, 301_600))),
            250,
        ),
    ];
    let mut all = Vec::new();
    let mut waits = Vec::new();
    for (source, path, blocks) in runs {
        let run = play(
            &mut prepare,
            &mut host,
            &mut disk,
            &mut clock,
            Some((source, path)),
            blocks,
        );
        all.extend_from_slice(&run.out);
        waits.push(run.waited as f32);
    }
    all.extend_from_slice(&waits);
    all.push(
        host.engine()
            .disk_stats()
            .underruns
            .load(core::sync::atomic::Ordering::Relaxed) as f32,
    );
    all
}

// ---------------------------------------------------------------- the audio clip player (AU-4)

/// A 16-bit WAV of `channels` channels at `rate`, `frames` frames, sample `(f, c)` given.
pub fn wav16(rate: u32, channels: u16, frames: u64, sample: impl Fn(u64, usize) -> i16) -> Vec<u8> {
    let block = u32::from(channels) * 2;
    let data = frames as usize * block as usize;
    let mut w = Vec::with_capacity(44 + data);
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data as u32).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&channels.to_le_bytes());
    w.extend_from_slice(&rate.to_le_bytes());
    w.extend_from_slice(&(rate * block).to_le_bytes());
    w.extend_from_slice(&(block as u16).to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&(data as u32).to_le_bytes());
    for f in 0..frames {
        for c in 0..usize::from(channels) {
            w.extend_from_slice(&sample(f, c).to_le_bytes());
        }
    }
    w
}

/// An engine with a disk in memory, rendering whole blocks of [`BLOCK`] frames and stepping
/// the disk after each (its clock is the rendered time): what the player's tests and the
/// parity scenario drive.
pub struct Rig {
    pub prepare: Prepare,
    pub host: Host,
    pub disk: MemDisk,
    /// Frames rendered.
    pub frames: u64,
    pub rate: u32,
}

impl Rig {
    /// An engine at `rate` with `disk`, after a `Reset` (epoch 1) and `setup`.
    pub fn new(rate: u32, disk: MemDisk, setup: &[ControlMsg]) -> Rig {
        let (prepare, host) = channel(rate as f32).expect("a channel");
        let mut rig = Rig {
            prepare,
            host,
            disk,
            frames: 0,
            rate,
        };
        rig.send(&[ControlMsg::Reset { epoch: 1 }]);
        rig.send(setup);
        rig
    }

    /// Submits `msgs` (epoch 1).
    pub fn send(&mut self, msgs: &[ControlMsg]) {
        let stream = encode_stream(1, 0, msgs).expect("encodes");
        self.prepare.submit(&stream).expect("accepted");
    }

    /// Renders `blocks` blocks; returns the left and right channels.
    pub fn render(&mut self, blocks: usize) -> (Vec<f32>, Vec<f32>) {
        let mut left = Vec::with_capacity(blocks * BLOCK);
        let mut right = Vec::with_capacity(blocks * BLOCK);
        for _ in 0..blocks {
            let n = self.host.render(BLOCK);
            left.extend_from_slice(&self.host.left()[..n]);
            right.extend_from_slice(&self.host.right()[..n]);
            self.frames += n as u64;
            let now = self.frames * 1000 / u64::from(self.rate.max(1));
            self.disk.step(&mut self.prepare, now);
            self.prepare.flush();
        }
        (left, right)
    }
}

/// Where clip 2 starts in the long source.
pub const LONG_LOOP_CLIP: u64 = 280_000;
/// Clip 2's loop, relative to its start: from 3 s in (past its preload of 1.5 s), 0.2 s long.
pub const LONG_LOOP: (u64, u64) = (3 * RATE as u64, 3 * RATE as u64 + 1_600);
/// Clip 1's gain.
pub const SHORT_GAIN: f32 = 0.5;

/// The player's standard setup at [`RATE`]: [`rig_stream`]'s audio track, sources and clips
/// (clip 0 on the long source from [`CLIP_START`], played once; clip 1 the short source with
/// a loop, at [`SHORT_GAIN`]), clip 2 a loop of the long source that its preload does not
/// cover, the three in scenes 0, 1 and 2 of the grid, a transport at 120 BPM that launches at
/// once.
pub fn player_setup() -> Vec<ControlMsg> {
    use crate::msg::SceneSlot;
    use crate::time::{Quantize, Tempo, TimeSignature};
    let mut msgs = vec![
        ControlMsg::SetTransport {
            tempo: Tempo::new(120.0).expect("a tempo"),
            signature: TimeSignature::default(),
            launch_quantize: Quantize::None,
        },
        ControlMsg::TrackCreate {
            track: TrackSlot(0),
            kind: TrackKind::Audio,
        },
        ControlMsg::AudioSourceDeclare {
            source: SourceSlot(0),
            sample_rate: RATE,
            channels: 2,
            frames: Frame(LONG_FRAMES),
            media: LONG,
        },
        ControlMsg::AudioSourceDeclare {
            source: SourceSlot(1),
            sample_rate: RATE,
            channels: 2,
            frames: Frame(SHORT_FRAMES),
            media: SHORT,
        },
        ControlMsg::ClipAudioSet {
            clip: ClipSlot(0),
            source: SourceSlot(0),
            start: Frame(CLIP_START),
            length: Frame(30 * RATE as u64),
            gain: 1.0,
            looping: None,
        },
        ControlMsg::ClipAudioSet {
            clip: ClipSlot(1),
            source: SourceSlot(1),
            start: Frame(0),
            length: Frame(SHORT_FRAMES),
            gain: SHORT_GAIN,
            looping: Some((Frame(1_000), Frame(1_800))),
        },
        ControlMsg::ClipAudioSet {
            clip: ClipSlot(2),
            source: SourceSlot(0),
            start: Frame(LONG_LOOP_CLIP),
            length: Frame(4 * RATE as u64),
            gain: 1.0,
            looping: Some((Frame(LONG_LOOP.0), Frame(LONG_LOOP.1))),
        },
    ];
    for (scene, clip) in [(0, 0), (1, 1), (2, 2)] {
        msgs.push(ControlMsg::SceneSetSlot {
            track: TrackSlot(0),
            scene: SceneSlot(scene),
            clip: Some(ClipSlot(clip)),
        });
    }
    msgs
}

/// The launch of the grid cell of `scene` on track 0.
pub fn launch(scene: u16) -> ControlMsg {
    ControlMsg::LaunchClip {
        track: TrackSlot(0),
        scene: crate::msg::SceneSlot(scene),
    }
}

/// Clip 0 placed whole at tick 0 of track 0's arrangement.
pub fn placement() -> ControlMsg {
    use crate::msg::{Placement, PlacementSlot, Span};
    ControlMsg::TrackPlacementsUpload {
        track: TrackSlot(0),
        first: 0,
        total: 1,
        placements: vec![Placement {
            placement: PlacementSlot(0),
            clip: ClipSlot(0),
            start: crate::time::Tick(0),
            span: Span::Frames {
                offset: Frame(0),
                length: Frame(30 * RATE as u64),
            },
        }],
    }
}

/// The player's scenario, for the parity test: clip 0 launched from the grid; clip 1 (a short
/// loop from memory); clip 0's placement entered by a seek into a spot its preload does not
/// cover (the ring's wait, then the gate); clip 2 (a loop through the ring); a stop. Returns
/// the interleaved output, then the underruns.
pub fn player_scenario() -> Vec<f32> {
    use crate::time::{PPQ, Tick};
    let mut rig = Rig::new(RATE, MemDisk::standard(), &player_setup());
    rig.render(200);
    let mut all = Vec::new();
    let mut take = |rig: &mut Rig, blocks: usize| {
        let (l, r) = rig.render(blocks);
        for (a, b) in l.iter().zip(&r) {
            all.push(*a);
            all.push(*b);
        }
    };
    rig.send(&[ControlMsg::Play { from: Tick(0) }, launch(0)]);
    take(&mut rig, 250);
    rig.send(&[launch(1)]);
    take(&mut rig, 100);
    // 12 s into the placement (120 BPM: 2 beats a second).
    rig.send(&[
        placement(),
        ControlMsg::BackToArrangement,
        ControlMsg::Seek {
            tick: Tick(24 * PPQ),
        },
    ]);
    take(&mut rig, 150);
    rig.send(&[launch(2)]);
    take(&mut rig, 250);
    rig.send(&[ControlMsg::StopTrack {
        track: TrackSlot(0),
    }]);
    take(&mut rig, 20);
    let underruns = rig
        .host
        .engine()
        .disk_stats()
        .underruns
        .load(core::sync::atomic::Ordering::Relaxed);
    // Resampled: the same sources in an engine at 11 025 Hz, clip 1 (a loop from memory)
    // then clip 0 through its preload and its ring.
    let mut rig = Rig::new(11_025, MemDisk::standard(), &player_setup());
    rig.render(200);
    rig.send(&[ControlMsg::Play { from: Tick(0) }, launch(1)]);
    take(&mut rig, 100);
    rig.send(&[launch(0)]);
    take(&mut rig, 200);
    all.push(underruns as f32);
    all
}
