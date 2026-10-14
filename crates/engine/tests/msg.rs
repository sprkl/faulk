//! The frozen parts of `engine::msg` (CT-3): header codec, tags, limits, sizes.

use engine::msg::{
    ACTIVE_CLIP_BYTES, AutomationPoint, AutomationTarget, ClipSlot, ControlMsg, DeviceKind,
    DeviceSlot, FEEDBACK_HEADER_BYTES, Feedback, HEADER_BYTES, Header, LaneSlot, MSG_ALIGN,
    MSG_VERSION, MsgError, Note, Opaque, ParamId, Placement, RECORDED_NOTE_WIRE_BYTES, RtMsg,
    SceneSlot, SourceSlot, TRACK_METER_BYTES, Tag, TrackKind, TrackSlot, limits,
};
use engine::time::{Frame, Quantize, Tempo, Tick, TimeSignature};

fn header(tag: Tag, len: u32) -> Header {
    Header {
        version: MSG_VERSION,
        tag,
        epoch: 3,
        len,
        seq: 0xDEAD_BEEF,
    }
}

#[test]
fn the_header_is_sixteen_little_endian_bytes_and_round_trips() {
    let h = header(Tag::ParamSet, 0x0001_0203);
    let bytes = h.encode();
    assert_eq!(bytes.len(), HEADER_BYTES);
    assert_eq!(
        bytes,
        [
            1, 0, // version, flags
            23, 0, // tag = ParamSet, little-endian
            3, 0, // epoch
            0, 0, // reserved
            3, 2, 1, 0, // len
            0xEF, 0xBE, 0xAD, 0xDE, // seq
        ]
    );
    assert_eq!(Header::decode(&bytes, 3), Ok(h));
}

/// One sample per `ControlMsg` variant: the enumeration of the contract.
fn samples() -> Vec<ControlMsg> {
    let t = TrackSlot(0);
    vec![
        ControlMsg::Reset { epoch: 1 },
        ControlMsg::SetTransport {
            tempo: Tempo::clamped(120.0),
            signature: TimeSignature::COMMON,
            launch_quantize: Quantize::Bar,
        },
        ControlMsg::SetMetronome {
            enabled: true,
            volume: 0.5,
            count_in_bars: 1,
        },
        ControlMsg::SetMaster {
            gain: 1.0,
            mute: false,
        },
        ControlMsg::TrackCreate {
            track: t,
            kind: TrackKind::Midi,
        },
        ControlMsg::TrackRemove { track: t },
        ControlMsg::TrackSetMix {
            track: t,
            gain: 1.0,
            pan: 0.0,
            mute: false,
        },
        ControlMsg::TrackSetSolo {
            track: t,
            solo: true,
        },
        ControlMsg::TrackArm {
            track: t,
            armed: true,
        },
        ControlMsg::TrackSetMonitor {
            track: t,
            monitor: true,
        },
        ControlMsg::SelectTrack { track: Some(t) },
        ControlMsg::DeviceInsert {
            track: t,
            device: DeviceSlot(0),
            kind: DeviceKind(1),
            state: vec![],
        },
        ControlMsg::DeviceRemove {
            track: t,
            device: DeviceSlot(0),
        },
        ControlMsg::DeviceSetEnabled {
            track: t,
            device: DeviceSlot(0),
            enabled: false,
        },
        ControlMsg::ParamSet {
            track: t,
            device: DeviceSlot(0),
            param: ParamId(0),
            value: 0.5,
        },
        ControlMsg::ClipNotesUpload {
            clip: ClipSlot(0),
            length: Tick(3840),
            looped: true,
            first: 0,
            total: 0,
            notes: vec![],
        },
        ControlMsg::ClipRemove { clip: ClipSlot(0) },
        ControlMsg::AudioSourceDeclare {
            source: SourceSlot(0),
            sample_rate: 48_000,
            channels: 2,
            frames: Frame(0),
        },
        ControlMsg::AudioSourceRelease {
            source: SourceSlot(0),
        },
        ControlMsg::ClipAudioSet {
            clip: ClipSlot(0),
            source: SourceSlot(0),
            start: Frame(0),
            length: Frame(0),
        },
        ControlMsg::TrackPlacementsUpload {
            track: t,
            first: 0,
            total: 1,
            placements: vec![Placement::default()],
        },
        ControlMsg::AutomationLaneUpload {
            lane: LaneSlot(0),
            track: t,
            target: AutomationTarget::MixerGain,
            first: 0,
            total: 0,
            points: vec![],
        },
        ControlMsg::AutomationLaneRemove { lane: LaneSlot(0) },
        ControlMsg::SceneSetSlot {
            track: t,
            scene: SceneSlot(0),
            clip: None,
        },
        ControlMsg::LaunchClip {
            track: t,
            scene: SceneSlot(0),
        },
        ControlMsg::LaunchScene {
            scene: SceneSlot(0),
        },
        ControlMsg::StopTrack { track: t },
        ControlMsg::StopAll,
        ControlMsg::BackToArrangement,
        ControlMsg::Play { from: Tick(0) },
        ControlMsg::Stop,
        ControlMsg::Seek { tick: Tick(0) },
        ControlMsg::SetLoop { region: None },
        ControlMsg::SetRecording { recording: true },
        ControlMsg::NoteOn {
            track: t,
            pitch: 60,
            velocity: 100,
        },
        ControlMsg::NoteOff {
            track: t,
            pitch: 60,
        },
        ControlMsg::AllNotesOff { track: t },
    ]
}

#[test]
fn every_tag_round_trips_and_every_variant_has_its_tag() {
    for tag in Tag::ALL {
        assert_eq!(Tag::from_u16(tag as u16), Some(tag));
        let bytes = header(tag, 0).encode();
        assert_eq!(Header::decode(&bytes, 3).map(|h| h.tag), Ok(tag));
    }
    let mut numbers: Vec<u16> = Tag::ALL.iter().map(|t| *t as u16).collect();
    numbers.sort_unstable();
    numbers.dedup();
    assert_eq!(numbers.len(), Tag::ALL.len(), "tags are unique");

    let samples = samples();
    assert_eq!(samples.len(), Tag::ALL.len(), "one sample per variant");
    let mut seen: Vec<Tag> = samples.iter().map(ControlMsg::tag).collect();
    seen.sort_by_key(|t| *t as u16);
    seen.dedup();
    let mut all = Tag::ALL.to_vec();
    all.sort_by_key(|t| *t as u16);
    assert_eq!(
        seen, all,
        "every variant has its own tag and every tag its variant"
    );
    for sample in &samples {
        assert_eq!(
            sample.check_limits(),
            Ok(()),
            "{sample:?} is within the limits"
        );
        assert_eq!(
            sample.tag().is_live(),
            matches!(
                sample,
                ControlMsg::NoteOn { .. }
                    | ControlMsg::NoteOff { .. }
                    | ControlMsg::AllNotesOff { .. }
            )
        );
    }
}

#[test]
fn a_bad_header_is_a_typed_error() {
    let good = header(Tag::Stop, 4).encode();
    assert_eq!(
        Header::decode(&good[..10], 3),
        Err(MsgError::Truncated {
            needed: HEADER_BYTES,
            available: 10
        })
    );
    assert_eq!(
        Header::decode(&[], 3),
        Err(MsgError::Truncated {
            needed: 16,
            available: 0
        })
    );

    let mut version = good;
    version[0] = 2;
    assert_eq!(
        Header::decode(&version, 3),
        Err(MsgError::UnsupportedVersion(2))
    );

    let mut flags = good;
    flags[1] = 1;
    assert_eq!(Header::decode(&flags, 3), Err(MsgError::Corrupt));
    let mut reserved = good;
    reserved[7] = 0x80;
    assert_eq!(Header::decode(&reserved, 3), Err(MsgError::Corrupt));

    let mut large = good;
    large[8..12].copy_from_slice(&(limits::MAX_CONTROL_MSG_BYTES + 1).to_le_bytes());
    assert_eq!(
        Header::decode(&large, 3),
        Err(MsgError::TooLarge(limits::MAX_CONTROL_MSG_BYTES + 1))
    );

    let mut unknown = good;
    unknown[2..4].copy_from_slice(&65535u16.to_le_bytes());
    assert_eq!(
        Header::decode(&unknown, 3),
        Err(MsgError::UnknownTag {
            tag: 65535,
            skip: 20
        })
    );
    // An unknown tag is skipped whatever its epoch (its length is what matters).
    assert!(matches!(
        Header::decode(&unknown, 4),
        Err(MsgError::UnknownTag { .. })
    ));
    assert_eq!(
        Header::decode(&good, 4),
        Err(MsgError::StaleEpoch {
            message: 3,
            current: 4
        })
    );
    // Only a Reset crosses epochs.
    let reset = header(Tag::Reset, 0).encode();
    assert_eq!(Header::decode(&reset, 4).map(|h| h.epoch), Ok(3));
}

#[test]
fn messages_in_a_stream_start_on_four_byte_boundaries() {
    assert_eq!(Header::frame_len(0), HEADER_BYTES);
    assert_eq!(Header::frame_len(1), HEADER_BYTES + MSG_ALIGN);
    assert_eq!(Header::frame_len(4), HEADER_BYTES + 4);
    assert_eq!(Header::frame_len(5), HEADER_BYTES + 8);
    assert_eq!(
        Header::frame_len(limits::MAX_CONTROL_MSG_BYTES),
        HEADER_BYTES + limits::MAX_CONTROL_MSG_BYTES as usize
    );
}

#[test]
fn limits_fit_under_the_wasm32_cap() {
    use limits::*;
    // The worst case of the tables: every one at its limit, each doubled (the next table
    // exists next to the old one during a growth), plus the largest table once more for
    // the fragmentation of a linear memory that never shrinks (freed blocks of
    // 1 + 2 + … + n/2 cannot hold 2n), the track buffers, the PCM budget, the rings.
    let notes = u64::from(MAX_NOTES) * size_of::<Note>() as u64;
    let points = u64::from(MAX_AUTOMATION_POINTS) * size_of::<AutomationPoint>() as u64;
    let placements = u64::from(MAX_PLACEMENTS) * size_of::<Placement>() as u64;
    let clips = u64::from(MAX_CLIPS) * 64;
    let lanes = u64::from(MAX_LANES) * 64;
    let sources = u64::from(MAX_AUDIO_SOURCES) * size_of::<Option<Opaque>>() as u64;
    // Per track: its entry, its mix state, two stereo block buffers for the chain, the
    // grid slots.
    let per_track = size_of::<Option<Opaque>>() as u64
        + 256
        + 4 * (MAX_BLOCK_FRAMES as u64) * 4
        + u64::from(MAX_SCENES) * 8;
    let tracks = u64::from(MAX_TRACKS) * per_track;
    let largest = notes.max(points).max(placements);
    let tables = notes + points + placements + clips + lanes + sources + tracks;
    let rings = 2 * (RT_RING_CAPACITY as u64) * size_of::<RtMsg>() as u64
        + (LIVE_QUEUE_CAPACITY as u64) * size_of::<RtMsg>() as u64;
    let inbox = u64::from(MAX_CONTROL_MSG_BYTES) * 2;
    let feedback = Feedback::max_size_for(MAX_TRACKS as usize) as u64;
    let total = 2 * tables + largest + rings + inbox + feedback;
    eprintln!("worst case of the tables: {} MiB", total >> 20);
    // The devices are not a count: their declared sizes count against the engine memory
    // budget, which the tables' worst case must leave room for.
    assert!(
        total < ENGINE_MEMORY_BUDGET_BYTES,
        "{} MiB of tables leave nothing for the devices in the engine budget",
        total >> 20
    );
    // The budgets fit under the wasm32 cap with the code, stack and data reserve.
    const {
        assert!(
            ENGINE_MEMORY_BUDGET_BYTES + PCM_BUDGET_BYTES + WASM_RESERVED_BYTES <= WASM_MAX_BYTES
        )
    };
    assert_eq!(
        u64::from(WASM_INITIAL_PAGES) * 65_536,
        16 << 20,
        "the initial memory is 16 MiB"
    );
}

#[test]
fn an_rt_message_is_small_and_sent_by_move() {
    // Moved through a ring slot: small enough that a slot copy is a few words.
    assert!(
        size_of::<RtMsg>() <= 64,
        "RtMsg is {} bytes",
        size_of::<RtMsg>()
    );
    assert!(
        size_of::<Note>() <= 24,
        "Note is {} bytes",
        size_of::<Note>()
    );
    assert_eq!(Feedback::size_for(0, 0, 0), FEEDBACK_HEADER_BYTES);
    assert_eq!(
        Feedback::size_for(2, 3, 4),
        FEEDBACK_HEADER_BYTES
            + 2 * TRACK_METER_BYTES
            + 3 * ACTIVE_CLIP_BYTES
            + 4 * RECORDED_NOTE_WIRE_BYTES
    );
    // The largest report of the largest session stays small enough for a WebSocket frame.
    assert!(Feedback::max_size_for(limits::MAX_TRACKS as usize) < 256 << 10);
}
