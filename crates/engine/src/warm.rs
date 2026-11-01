//! Warm-up of the apply path (#355): runs every message of the engine once on a throwaway
//! channel, so that a host whose code is compiled lazily (a WebAssembly engine in V8: a
//! function is compiled at its first call) pays that cost before the first sound, not in
//! the first block that applies a message.
//!
//! The channel built here is private: it shares nothing with the channel the host plays (no
//! ring, no table, no output buffer, no counter), and it is dropped before [`warm_up`]
//! returns. Nothing it renders leaves this module. It allocates, as [`crate::host::channel`]
//! does: call it from a setup thread (the worklet's message handler), never from the audio
//! callback.
//!
//! The sequence follows the order of [`crate::msg::ControlMsg`]: the structure (tracks, the
//! devices of the registry, clips, an audio source, an arrangement, automation lanes, the
//! launch grid), the mix and parameters, the transport with sounding notes, then the
//! removals and a second `Reset` (the swap of the whole state). [`messages`] lists them;
//! a test proves that every variant of [`Tag`] is in it.

use crate::device::DeviceRole;
use crate::devices::REGISTRY;
use crate::host::channel;
use crate::msg::{
    AutomationPoint, AutomationTarget, ClipSlot, ControlMsg as M, Curve, DeviceSlot, LaneSlot,
    MsgError, Note, ParamId, Placement, PlacementSlot, SceneSlot, SourceSlot, Span, Tag, TrackKind,
    TrackSlot, encode_stream, limits,
};
use crate::time::{Frame, LoopRegion, Quantize, Tempo, Tick, TimeSignature};

/// Frames of one warm-up block (a Web Audio render quantum).
const BLOCK: usize = 128;

/// Blocks rendered after most phases.
const BLOCKS: u32 = 4;

/// Blocks rendered after the transport phase: at the tempo of the warm-up (the fastest, 999
/// BPM) a beat is 2 883 frames at 48 kHz, so 40 blocks (5 120 frames) cross a beat boundary
/// (the quantized launch fires), the end of a clip's notes and the wrap of its one-beat loop
/// (a test checks the wrap). At a higher sample rate the same blocks reach less far: the beat
/// is then not crossed, which costs only the compilation of those paths in the first block
/// that reaches them.
const PLAY_BLOCKS: u32 = 40;

/// Messages sent together, then applied and rendered over `blocks` blocks.
#[derive(Clone, Debug)]
pub struct Phase {
    pub messages: Vec<M>,
    pub blocks: u32,
}

/// What a warm-up did, for the tests and the caller's diagnosis.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WarmUp {
    /// Messages sent.
    pub sent: u32,
    /// Rt messages the throwaway engine applied.
    pub applied: u64,
    /// Messages the prepare side rejected.
    pub rejected: u32,
    /// Rt messages the audio side skipped (a slot that did not exist).
    pub skipped: u32,
    /// Blocks rendered.
    pub blocks: u32,
    /// Largest sample the throwaway host rendered (it proves the devices sounded; nothing of
    /// it leaves this module).
    pub peak: f32,
    /// Times the playhead went back while the clock ran (the loop wrapped).
    pub loop_wraps: u32,
    /// Lowest playhead tick seen (negative during the count-in of a recording).
    pub lowest_tick: i64,
}

/// The phases of the warm-up: each is sent, applied by pumps and rendered over a few blocks
/// before the next (a message can name what an earlier phase created). Epoch 1 for all but
/// the last phase, which opens epoch 2.
pub fn messages() -> Vec<Phase> {
    let t = TrackSlot;
    let d = DeviceSlot;
    let synth_track = t(0);
    let audio_track = t(1);

    // Phase 1: structure.
    let mut structure = vec![
        M::Reset { epoch: 1 },
        M::SetTransport {
            tempo: Tempo::clamped(Tempo::MAX),
            signature: TimeSignature::COMMON,
            launch_quantize: Quantize::Beat,
        },
        M::SetMetronome {
            enabled: true,
            volume: 1.0,
            count_in_bars: 1,
        },
        M::SetMaster {
            gain: 1.0,
            pan: 0.0,
            mute: false,
        },
        M::TrackCreate {
            track: synth_track,
            kind: TrackKind::Midi,
        },
        M::TrackCreate {
            track: audio_track,
            kind: TrackKind::Audio,
        },
    ];
    // The devices of the registry: the first instrument on the first track, any other
    // instrument on a MIDI track of its own, the effects in the chain of the first track.
    let mut next_track = 2;
    let mut next_effect = 1;
    let mut first_instrument = true;
    let mut placed = Vec::new();
    for builtin in REGISTRY {
        let desc = builtin.descriptor;
        let (track, device) = match desc.role {
            DeviceRole::Instrument if first_instrument => {
                first_instrument = false;
                (synth_track, d(0))
            }
            DeviceRole::Instrument => {
                let track = t(next_track);
                next_track += 1;
                structure.push(M::TrackCreate {
                    track,
                    kind: TrackKind::Midi,
                });
                (track, d(0))
            }
            DeviceRole::Effect => {
                let device = d(next_effect);
                next_effect += 1;
                (synth_track, device)
            }
            DeviceRole::MidiEffect => continue,
        };
        structure.push(M::DeviceInsert {
            track,
            device,
            kind: desc.kind_id,
            state: Vec::new(),
        });
        placed.push((track, device, desc));
    }
    // The effects of the registry again, in the master's chain (#364): its own row, its own
    // path through the graph.
    let mut master_placed = Vec::new();
    for builtin in REGISTRY {
        let desc = builtin.descriptor;
        if desc.role != DeviceRole::Effect {
            continue;
        }
        let device = d(master_placed.len() as u16);
        structure.push(M::DeviceInsert {
            track: TrackSlot::MASTER,
            device,
            kind: desc.kind_id,
            state: Vec::new(),
        });
        master_placed.push((TrackSlot::MASTER, device, desc));
    }
    structure.extend([
        M::ClipNotesUpload {
            clip: ClipSlot(0),
            length: Tick(480),
            looping: LoopRegion::new(Tick(0), Tick(480)),
            first: 0,
            total: 2,
            notes: vec![
                Note {
                    start: Tick(0),
                    length: Tick(120),
                    pitch: 60,
                    velocity: 100,
                    ..Note::default()
                },
                Note {
                    start: Tick(240),
                    length: Tick(120),
                    pitch: 64,
                    velocity: 90,
                    ..Note::default()
                },
            ],
        },
        M::AudioSourceDeclare {
            source: SourceSlot(0),
            sample_rate: 48_000,
            channels: 1,
            frames: Frame::new(BLOCK as u64),
        },
        M::ClipAudioSet {
            clip: ClipSlot(1),
            source: SourceSlot(0),
            start: Frame::ZERO,
            length: Frame::new(BLOCK as u64),
        },
        M::TrackPlacementsUpload {
            track: synth_track,
            first: 0,
            total: 1,
            placements: vec![Placement {
                placement: PlacementSlot(0),
                clip: ClipSlot(0),
                start: Tick(0),
                span: Span::Ticks {
                    offset: Tick(0),
                    length: Tick(480),
                },
            }],
        },
        M::TrackPlacementsUpload {
            track: audio_track,
            first: 0,
            total: 1,
            placements: vec![Placement {
                placement: PlacementSlot(1),
                clip: ClipSlot(1),
                start: Tick(0),
                span: Span::Frames {
                    offset: Frame::ZERO,
                    length: Frame::new(BLOCK as u64),
                },
            }],
        },
        M::AutomationLaneUpload {
            lane: LaneSlot(0),
            track: audio_track,
            target: AutomationTarget::MixerGain,
            first: 0,
            total: 2,
            points: vec![
                AutomationPoint {
                    tick: Tick(0),
                    value: 1.0,
                    curve: Curve::Linear,
                },
                AutomationPoint {
                    tick: Tick(240),
                    value: 0.5,
                    curve: Curve::Step,
                },
            ],
        },
        M::SceneSetSlot {
            track: synth_track,
            scene: SceneSlot(0),
            clip: Some(ClipSlot(0)),
        },
    ]);
    if let Some((track, device, desc)) = placed.first().copied()
        && let Some(param) = desc.params.first()
    {
        structure.push(M::AutomationLaneUpload {
            lane: LaneSlot(1),
            track,
            target: AutomationTarget::DeviceParam {
                device,
                param: param.id,
            },
            first: 0,
            total: 1,
            points: vec![AutomationPoint {
                tick: Tick(0),
                value: param.default as f32,
                curve: Curve::Step,
            }],
        });
    }

    // The master's lanes (#378): its fader, and the first parameter of its first effect. The
    // render of a lane on the master is its own path through the graph.
    structure.push(M::AutomationLaneUpload {
        lane: LaneSlot(2),
        track: TrackSlot::MASTER,
        target: AutomationTarget::MixerGain,
        first: 0,
        total: 2,
        points: vec![
            AutomationPoint {
                tick: Tick(0),
                value: 0.0,
                curve: Curve::Linear,
            },
            AutomationPoint {
                tick: Tick(240),
                value: -6.0,
                curve: Curve::Step,
            },
        ],
    });
    if let Some((track, device, desc)) = master_placed.first().copied()
        && let Some(param) = desc.params.first()
    {
        structure.push(M::AutomationLaneUpload {
            lane: LaneSlot(3),
            track,
            target: AutomationTarget::DeviceParam {
                device,
                param: param.id,
            },
            first: 0,
            total: 1,
            points: vec![AutomationPoint {
                tick: Tick(0),
                value: param.default as f32,
                curve: Curve::Step,
            }],
        });
    }

    // Phase 2: the mix, and every parameter of every device at its default.
    let mut mix = vec![
        M::TrackSetMix {
            track: synth_track,
            gain: 0.8,
            pan: -0.25,
            mute: false,
        },
        M::TrackSetSolo {
            track: synth_track,
            solo: true,
        },
        M::TrackSetSolo {
            track: synth_track,
            solo: false,
        },
        M::TrackArm {
            track: synth_track,
            armed: true,
        },
        M::TrackSetMonitor {
            track: synth_track,
            monitor: true,
        },
        M::SelectTrack {
            track: Some(synth_track),
        },
        M::SetMaster {
            gain: 0.9,
            pan: 0.1,
            mute: false,
        },
        M::HoldMix {
            track: synth_track,
            gain: Some(1.0),
            pan: Some(0.0),
        },
        M::HoldMix {
            track: TrackSlot::MASTER,
            gain: Some(1.0),
            pan: Some(0.0),
        },
    ];
    for (track, device, desc) in placed.iter().chain(&master_placed) {
        mix.push(M::DeviceSetEnabled {
            track: *track,
            device: *device,
            enabled: false,
        });
        mix.push(M::DeviceSetEnabled {
            track: *track,
            device: *device,
            enabled: true,
        });
        for param in desc.params {
            mix.push(M::ParamSet {
                track: *track,
                device: *device,
                param: ParamId(param.id.0),
                value: param.default as f32,
            });
        }
    }

    // Phase 3: the transport, with notes sounding.
    let mut play = vec![
        M::SetLoop {
            region: LoopRegion::new(Tick(0), Tick(480)),
        },
        M::Play { from: Tick(0) },
        M::LaunchClip {
            track: synth_track,
            scene: SceneSlot(0),
        },
        M::LaunchScene {
            scene: SceneSlot(0),
        },
        M::NoteOn {
            track: synth_track,
            pitch: 64,
            velocity: 90,
        },
    ];
    // A note on every instrument, at the first key it names (a drum kit) or middle C.
    for (track, _, desc) in &placed {
        if desc.role == DeviceRole::Instrument {
            play.push(M::NoteOn {
                track: *track,
                pitch: desc.named_keys.first().map_or(60, |k| k.key),
                velocity: 100,
            });
        }
    }
    // A recording from a stopped clock with the metronome on starts with a count-in bar.
    let halt = vec![M::Stop];
    let record = vec![
        M::SetRecording { recording: true },
        M::Play { from: Tick(0) },
    ];
    let seek = vec![
        M::Seek { tick: Tick(120) },
        M::NoteOff {
            track: synth_track,
            pitch: 60,
        },
        M::AllNotesOff { track: synth_track },
        M::SetRecording { recording: false },
        M::StopTrack { track: synth_track },
        M::StopAll,
        M::BackTrackToArrangement { track: synth_track },
        M::BackToArrangement,
        M::Stop,
        M::AllSoundOff,
    ];

    // Phase 4: the removals.
    let mut removals = vec![
        M::SelectTrack { track: None },
        M::AutomationLaneRemove { lane: LaneSlot(0) },
        M::AutomationLaneRemove { lane: LaneSlot(2) },
        M::ClipRemove { clip: ClipSlot(0) },
        M::ClipRemove { clip: ClipSlot(1) },
        M::AudioSourceRelease {
            source: SourceSlot(0),
        },
    ];
    for (track, device, _) in master_placed.iter().chain(placed.iter().rev()) {
        removals.push(M::DeviceRemove {
            track: *track,
            device: *device,
        });
    }
    for slot in (0..next_track).rev() {
        removals.push(M::TrackRemove { track: t(slot) });
    }

    // Phase 5: the swap of the whole state.
    let swap = vec![M::Reset { epoch: 2 }];
    let phase = |messages, blocks| Phase { messages, blocks };
    vec![
        phase(structure, BLOCKS),
        phase(mix, BLOCKS),
        phase(play, PLAY_BLOCKS),
        phase(halt, BLOCKS),
        phase(record, BLOCKS),
        phase(seek, BLOCKS),
        phase(removals, BLOCKS),
        phase(swap, BLOCKS),
    ]
}

/// Runs [`messages`] through a private channel at `sample_rate` and renders a few blocks
/// after each phase. Returns what happened, or the error that stopped a stream.
pub fn warm_up(sample_rate: f32) -> Result<WarmUp, MsgError> {
    let (mut prepare, mut host) = channel(sample_rate)?;
    let mut report = WarmUp::default();
    let mut seq = 0u32;
    let mut epoch = 1u16;
    for Phase { messages, blocks } in messages() {
        let phase = messages;
        if let Some(M::Reset { epoch: next }) = phase.first() {
            epoch = *next;
        }
        let bytes = encode_stream(epoch, seq, &phase)?;
        seq = seq.wrapping_add(phase.len() as u32);
        report.sent += phase.len() as u32;
        prepare.submit(&bytes)?;
        // The host applies `MAX_APPLY_PER_BLOCK` messages a block: pump until the ring is empty.
        for _ in 0..=(phase.len() / limits::MAX_APPLY_PER_BLOCK) {
            host.pump();
        }
        for _ in 0..blocks {
            let before = host.engine().clock().playhead_tick().0;
            host.render(BLOCK);
            let after = host.engine().clock().playhead_tick().0;
            if host.engine().clock().is_running() && after < before && after >= 0 {
                report.loop_wraps += 1;
            }
            report.lowest_tick = report.lowest_tick.min(after);
            report.blocks += 1;
            for sample in host.left().iter().chain(host.right()) {
                report.peak = report.peak.max(sample.abs());
            }
        }
        prepare.flush();
    }
    report.applied = host.engine().applied();
    report.rejected = prepare.preparer().rejected();
    report.skipped = host.engine().skipped();
    Ok(report)
}

/// Every [`Tag`] the warm-up sends.
pub fn tags() -> Vec<Tag> {
    messages()
        .iter()
        .flat_map(|p| &p.messages)
        .map(M::tag)
        .collect()
}
