//! A stand-in host of the prepare/apply cycle, for tests only (feature `testing`): the two
//! sides of [`channel`] on one thread, the test devices as factories, and the graph's
//! render (`Engine::process`, MO-3) so that a test can listen to what `apply` built. The
//! native tests and the web module's test build (`engine-wasm`, feature `engine-cycle`)
//! run this same code, which is what makes their outputs comparable. It never allocates
//! on the audio side.

use super::{Engine, Preparer, channel};
use crate::device::DeviceSetup;
use crate::msg::{MsgError, TrackSlot, limits};

/// The two sides of the channel and the output buffers.
pub struct Cycle {
    pub preparer: Preparer,
    pub engine: Engine,
    left: Box<[f32]>,
    right: Box<[f32]>,
    inbox: Vec<u8>,
}

fn buffer(frames: usize) -> Result<Box<[f32]>, MsgError> {
    let mut v = Vec::new();
    v.try_reserve_exact(frames)
        .map_err(|_| MsgError::OutOfMemory { bytes: frames * 4 })?;
    v.resize(frames, 0.0);
    Ok(v.into_boxed_slice())
}

impl Cycle {
    /// Both sides at `sample_rate`, blocks up to [`limits::MAX_BLOCK_FRAMES`], the test
    /// devices as factories.
    pub fn new(sample_rate: f32) -> Result<Self, MsgError> {
        let setup = DeviceSetup {
            sample_rate,
            max_block_frames: limits::MAX_BLOCK_FRAMES,
        };
        let (preparer, engine) = channel(setup, crate::device::testing::factory)?;
        Ok(Cycle {
            preparer,
            engine,
            left: buffer(limits::MAX_BLOCK_FRAMES)?,
            right: buffer(limits::MAX_BLOCK_FRAMES)?,
            inbox: Vec::new(),
        })
    }

    /// A byte buffer of `len` bytes the host writes a stream into (fallible: `None` when
    /// the memory is refused).
    pub fn inbox(&mut self, len: usize) -> Option<&mut [u8]> {
        if len > self.inbox.len() {
            self.inbox.try_reserve_exact(len - self.inbox.len()).ok()?;
        }
        self.inbox.resize(len, 0);
        Some(&mut self.inbox[..])
    }

    /// Submits the first `len` bytes of the inbox ([`Cycle::submit`]).
    pub fn submit_inbox(&mut self, len: usize) -> Result<(), MsgError> {
        let inbox = core::mem::take(&mut self.inbox);
        let result = self.submit(&inbox[..len.min(inbox.len())]);
        self.inbox = inbox;
        result
    }

    /// The prepare side: submits a stream, then pushes what waited.
    pub fn submit(&mut self, stream: &[u8]) -> Result<(), MsgError> {
        let result = self.preparer.submit(stream);
        self.preparer.flush();
        result
    }

    /// The audio side: one block of `frames` (clamped to the buffers) through the graph
    /// (`Engine::process`). Returns the frames rendered.
    pub fn render(&mut self, frames: usize) -> usize {
        let frames = frames.min(self.left.len());
        self.engine
            .process(&mut self.left[..frames], &mut self.right[..frames]);
        frames
    }

    /// The left channel of the last block (its first `frames` samples are valid).
    pub fn left(&self) -> &[f32] {
        &self.left
    }

    /// The right channel of the last block.
    pub fn right(&self) -> &[f32] {
        &self.right
    }

    /// A counter, by number: 0 rejected, 1 skipped, 2 leaked, 3 pending, 4 track capacity,
    /// 5 tracks held, 6 clip capacity, 7 lane capacity, 8 source capacity, 9 applied (low
    /// 32 bits); `u32::MAX` for an unknown number. The same numbers in every build.
    pub fn counter(&self, which: u32) -> u32 {
        let s = self.engine.state();
        let n = match which {
            0 => self.preparer.rejected() as u64,
            1 => u64::from(self.engine.skipped()),
            2 => u64::from(self.engine.leaked()),
            3 => self.preparer.pending() as u64,
            4 => s.tracks.capacity() as u64,
            5 => s.tracks.as_slice().iter().flatten().count() as u64,
            6 => s.clips.capacity() as u64,
            7 => s.lanes.capacity() as u64,
            8 => s.sources.capacity() as u64,
            9 => self.engine.applied(),
            _ => return u32::MAX,
        };
        n as u32
    }
}

// ---------------------------------------------------------------- the scenario

/// Frames of each block of [`scenario`].
pub const SCENARIO_BLOCK: usize = 128;

/// Sample rate of [`scenario`].
pub const SCENARIO_RATE: f32 = 48_000.0;

/// Clip of the large upload of [`scenario`]: beyond the initial clip directory.
pub const SCENARIO_CLIP: u32 = 5000;

/// Notes of the large upload of [`scenario`], sent in [`SCENARIO_CHUNKS`] chunks.
pub const SCENARIO_NOTES: u32 = 300_000;

/// Chunks of the large upload.
pub const SCENARIO_CHUNKS: u32 = 3;

/// One step of [`scenario`]: a stream for the prepare side, or blocks to render.
#[derive(Clone, Debug)]
pub enum Step {
    Submit(Vec<u8>),
    Render(usize),
}

/// A held note on track 0, then, between rendered blocks, a growth of every runtime table
/// (tracks 16 → 64, the device row of track 0 4 → 16, its scene row 8 → 32, the clip
/// directory with a 300 000-note upload in three chunks, the lane directory, the source
/// table), then the graph's mixes, a MIDI effect, removals and a solo, then the release. The tone must sound without a gap from its first block to its
/// release. The same steps run natively and in the web module (parity).
pub fn scenario() -> Result<Vec<Step>, MsgError> {
    use crate::device::testing::{GAIN, TONE, gain, tone};
    use crate::msg::{
        AutomationPoint, AutomationTarget, ClipSlot, ControlMsg as M, Curve, DeviceSlot, LaneSlot,
        Note, SceneSlot, SourceSlot, TrackKind, encode_state, encode_stream,
    };
    use crate::time::{Frame, Tick};

    let epoch = 1;
    let mut seq = 0u32;
    let mut steps = Vec::new();
    let mut submit = |msgs: &[M], steps: &mut Vec<Step>| -> Result<(), MsgError> {
        let bytes = encode_stream(epoch, seq, msgs)?;
        seq = seq.wrapping_add(msgs.len() as u32);
        steps.push(Step::Submit(bytes));
        Ok(())
    };
    let track = |slot: u32| TrackSlot(slot);
    let tone_insert = |slot: u32, level: f32| M::DeviceInsert {
        track: track(slot),
        device: DeviceSlot(0),
        kind: TONE.kind_id,
        state: encode_state(&[(tone::GAIN, level)]),
    };

    submit(
        &[
            M::Reset { epoch },
            M::SetMaster {
                gain: 0.8,
                mute: false,
            },
            M::TrackCreate {
                track: track(0),
                kind: TrackKind::Midi,
            },
            tone_insert(0, 0.5),
            M::DeviceInsert {
                track: track(0),
                device: DeviceSlot(1),
                kind: GAIN.kind_id,
                state: encode_state(&[(gain::GAIN, 0.9)]),
            },
            M::NoteOn {
                track: track(0),
                pitch: 57,
                velocity: 100,
            },
        ],
        &mut steps,
    )?;
    steps.push(Step::Render(8));

    // The track table grows from 16 to 64 slots while the note plays; every other new
    // track plays a quiet note too.
    for slot in 1..48u32 {
        let mut msgs = Vec::new();
        msgs.push(M::TrackCreate {
            track: track(slot),
            kind: TrackKind::Midi,
        });
        msgs.push(tone_insert(slot, 0.05));
        if slot % 2 == 0 {
            msgs.push(M::NoteOn {
                track: track(slot),
                pitch: 60 + (slot % 12) as u8,
                velocity: 60,
            });
        }
        submit(&msgs, &mut steps)?;
        steps.push(Step::Render(1));
    }

    // A large clip upload, chunk by chunk, beyond the initial clip directory.
    let per_chunk = SCENARIO_NOTES / SCENARIO_CHUNKS;
    for chunk in 0..SCENARIO_CHUNKS {
        let first = chunk * per_chunk;
        let notes = (first..first + per_chunk)
            .map(|i| Note {
                start: Tick(i64::from(i) * 120),
                length: Tick(120),
                pitch: (i % 128) as u8,
                velocity: 90,
                ..Note::default()
            })
            .collect();
        submit(
            &[M::ClipNotesUpload {
                clip: ClipSlot(SCENARIO_CLIP),
                length: Tick(i64::from(SCENARIO_NOTES) * 120),
                looping: None,
                first,
                total: SCENARIO_NOTES,
                notes,
            }],
            &mut steps,
        )?;
        steps.push(Step::Render(2));
    }

    // The device row of track 0 (4 → 16 slots), its scene row (8 → 32), the lane
    // directory, the source table (16 → 128).
    let mut msgs = Vec::new();
    for device in 2..10u16 {
        msgs.push(M::DeviceInsert {
            track: track(0),
            device: DeviceSlot(device),
            kind: GAIN.kind_id,
            state: encode_state(&[(gain::GAIN, 1.0)]),
        });
    }
    msgs.push(M::SceneSetSlot {
        track: track(0),
        scene: SceneSlot(20),
        clip: Some(ClipSlot(SCENARIO_CLIP)),
    });
    msgs.push(M::AutomationLaneUpload {
        lane: LaneSlot(3000),
        track: track(0),
        target: AutomationTarget::MixerGain,
        first: 0,
        total: 2,
        points: Vec::from([
            AutomationPoint {
                tick: Tick(0),
                value: 0.5,
                curve: Curve::Linear,
            },
            AutomationPoint {
                tick: Tick(960),
                value: 1.0,
                curve: Curve::Step,
            },
        ]),
    });
    msgs.push(M::AudioSourceDeclare {
        source: SourceSlot(100),
        sample_rate: 48_000,
        channels: 2,
        frames: Frame(48_000),
    });
    submit(&msgs, &mut steps)?;
    steps.push(Step::Render(8));

    // The graph (MO-3): a MIDI effect before track 3's tone, mixes (pan, gain, mute),
    // tracks 40 to 47 removed while they play and slot 40 created again, then a cumulative
    // solo that keeps track 0 playing.
    use crate::device::testing::{TRANSPOSE, transpose};
    let mut msgs = vec![
        M::DeviceInsert {
            track: track(3),
            device: DeviceSlot(1),
            kind: TONE.kind_id,
            state: encode_state(&[(tone::GAIN, 0.3)]),
        },
        M::DeviceInsert {
            track: track(3),
            device: DeviceSlot(0),
            kind: TRANSPOSE.kind_id,
            state: encode_state(&[(transpose::SEMITONES, 12.0)]),
        },
        M::NoteOn {
            track: track(3),
            pitch: 64,
            velocity: 90,
        },
        M::TrackSetMix {
            track: track(0),
            gain: 0.9,
            pan: -0.5,
            mute: false,
        },
        M::TrackSetMix {
            track: track(2),
            gain: 1.0,
            pan: 0.0,
            mute: true,
        },
    ];
    for slot in 40..48u32 {
        msgs.push(M::TrackRemove { track: track(slot) });
    }
    msgs.push(M::TrackCreate {
        track: track(40),
        kind: TrackKind::Midi,
    });
    msgs.push(tone_insert(40, 0.1));
    msgs.push(M::NoteOn {
        track: track(40),
        pitch: 76,
        velocity: 80,
    });
    submit(&msgs, &mut steps)?;
    steps.push(Step::Render(6));
    let mut msgs = Vec::new();
    for slot in [0, 3] {
        msgs.push(M::TrackSetSolo {
            track: track(slot),
            solo: true,
        });
    }
    submit(&msgs, &mut steps)?;
    steps.push(Step::Render(6));

    // The release: every note off, then silence.
    let mut msgs = Vec::new();
    msgs.push(M::NoteOff {
        track: track(0),
        pitch: 57,
    });
    msgs.push(M::AllNotesOff { track: track(3) });
    for slot in (2..48u32).step_by(2) {
        msgs.push(M::AllNotesOff { track: track(slot) });
    }
    submit(&msgs, &mut steps)?;
    steps.push(Step::Render(8));
    Ok(steps)
}

/// Blocks of [`scenario`] rendered before the release: the tone must sound in each.
pub fn scenario_held_blocks(steps: &[Step]) -> usize {
    let renders: Vec<usize> = steps
        .iter()
        .filter_map(|s| match s {
            Step::Render(n) => Some(*n),
            Step::Submit(_) => None,
        })
        .collect();
    renders[..renders.len().saturating_sub(1)].iter().sum()
}
