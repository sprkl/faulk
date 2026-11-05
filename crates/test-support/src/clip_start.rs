//! A clip played from bar 1 on audio tracks with and without effects (#759): two audio tracks,
//! each a grid clip of a source of impulses launched with the `Play`, rendered offline. The
//! file must not depend on the composition of the tracks' chains: a transparent effect on a
//! track leaves every sample as it is. The engine's own test runs it under the allocation
//! detector (`crates/engine/tests/it/offline.rs`), the WASM parity in
//! `crates/engine-wasm/tests/export_parity.rs`.

use commands::export::{ExportBitDepth, ExportJob, ExportPlan, ExportRegion, ExportSettings};
use engine::msg::{
    ClipSlot, ControlMsg as M, DeviceSlot, ParamId, SceneSlot, SourceSlot, TrackKind, TrackSlot,
    encode_state, encode_stream,
};
use engine::time::{Frame, Quantize, Tempo, Tick, TimeSignature};

use crate::render::Disk;

const EPOCH: u16 = 1;

/// The media of the impulses' source.
pub const IMPULSES: [u8; 32] = [0x75; 32];

/// The impulses of the source, `(frame, level)`: on the clip's second frame (its first is the
/// start of the clip player's fade in, at a gain of 0), on frame 120 (the issue's: inside the
/// 5 ms fade of a track's strip, past the clip player's 2 ms one at 44.1 and 48 kHz), 3 ms in
/// (the same at every rate), one inside and one on its last frame.
pub fn impulses(rate: u32) -> [(u64, i16); 5] {
    let frames = impulse_frames(rate);
    [
        (1, 8_192),
        (120, 12_000),
        (u64::from(rate) * 3 / 1000, 10_000),
        (frames / 3, 4_096),
        (frames - 1, 8_192),
    ]
}

/// The source's length: half a second, a short source kept whole in memory.
pub fn impulse_frames(rate: u32) -> u64 {
    u64::from(rate) / 2
}

/// A mono 16-bit WAV of silence with [`impulses`].
pub fn impulse_wav(rate: u32) -> Vec<u8> {
    let frames = impulse_frames(rate);
    let at = impulses(rate);
    let data = frames as u32 * 2;
    let mut w = Vec::with_capacity(44 + data as usize);
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&rate.to_le_bytes());
    w.extend_from_slice(&(rate * 2).to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&data.to_le_bytes());
    for f in 0..frames {
        let v = at.iter().find(|(i, _)| *i == f).map_or(0, |(_, v)| *v);
        w.extend_from_slice(&v.to_le_bytes());
    }
    w
}

/// The project folder's files: the impulses' blob.
pub fn impulse_disk(rate: u32) -> Disk {
    let mut disk = Disk::default();
    disk.blob(IMPULSES, impulse_wav(rate));
    disk
}

/// An Echo at `device` of `track` that leaves its input as it is, bit for bit: no feedback,
/// no wet signal (`dry × 1 + wet × 0`).
pub fn transparent(track: TrackSlot, device: u16) -> M {
    use engine::devices::echo::{DESCRIPTOR, FEEDBACK, MIX};
    M::DeviceInsert {
        track,
        device: DeviceSlot(device),
        kind: DESCRIPTOR.kind_id,
        state: encode_state(&[(ParamId(FEEDBACK), 0.0), (ParamId(MIX), 0.0)]),
    }
}

/// The setup of two audio tracks that play the impulses from bar 1 (each its grid clip,
/// launched with the `Play`), with `devices` inserted after them; the start that plays them
/// (`Play` and the launches); the stop.
pub fn impulse_streams(rate: u32, devices: &[M]) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let frames = impulse_frames(rate);
    let mut setup = vec![
        M::Reset { epoch: EPOCH },
        M::SetTransport {
            tempo: Tempo::new(120.0).unwrap(),
            signature: TimeSignature::default(),
            launch_quantize: Quantize::None,
        },
        M::AudioSourceDeclare {
            source: SourceSlot(0),
            sample_rate: rate,
            channels: 1,
            frames: Frame(frames),
            media: IMPULSES,
        },
    ];
    let mut start = vec![M::Play { from: Tick(0) }];
    for slot in 0..2 {
        setup.extend([
            M::TrackCreate {
                track: TrackSlot(slot),
                kind: TrackKind::Audio,
            },
            M::ClipAudioSet {
                clip: ClipSlot(slot),
                source: SourceSlot(0),
                start: Frame(0),
                length: Frame(frames),
                gain: 1.0,
                looping: None,
            },
            M::SceneSetSlot {
                track: TrackSlot(slot),
                scene: SceneSlot(0),
                clip: Some(ClipSlot(slot)),
            },
        ]);
        start.push(M::LaunchClip {
            track: TrackSlot(slot),
            scene: SceneSlot(0),
        });
    }
    setup.extend_from_slice(devices);
    (
        encode_stream(EPOCH, 0, &setup).unwrap(),
        encode_stream(EPOCH, 1, &start).unwrap(),
        encode_stream(EPOCH, 2, &[M::Stop]).unwrap(),
    )
}

/// The export job of the impulses' song at `rate` in 32-bit floats: its body is the clip.
pub fn impulse_job(rate: u32, devices: &[M]) -> ExportJob {
    let (setup, start, stop) = impulse_streams(rate, devices);
    let body_frames = impulse_frames(rate);
    ExportJob {
        plan: ExportPlan {
            settings: ExportSettings {
                region: ExportRegion::Song,
                bit_depth: ExportBitDepth::Float32,
                sample_rate: rate,
                from_bar: None,
                to_bar: None,
            },
            start_tick: Tick(0),
            end_tick: Tick(0),
            body_frames,
            duration_secs: body_frames as f64 / f64::from(rate),
        },
        tracks: Vec::new(),
        setup,
        start,
        stop,
    }
}

/// The chains the tests compare with the chain without a device: a transparent effect on
/// track 0, on track 1, on both.
pub fn transparent_chains() -> [(&'static str, Vec<M>); 3] {
    [
        ("on track 0", vec![transparent(TrackSlot(0), 0)]),
        ("on track 1", vec![transparent(TrackSlot(1), 0)]),
        (
            "on both tracks",
            vec![transparent(TrackSlot(0), 0), transparent(TrackSlot(1), 0)],
        ),
    ]
}
