//! The frozen parts of `engine::msg` (CT-3): header codec, tags, limits, sizes.

use engine::msg::{
    ControlMsg, FEEDBACK_HEADER_BYTES, Feedback, HEADER_BYTES, Header, MSG_ALIGN, MSG_VERSION,
    MsgError, Note, RtMsg, Tag, TrackKind, TrackSlot, limits,
};

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
    let h = header(Tag::ParamSet, 0x0102_0304);
    let bytes = h.encode();
    assert_eq!(bytes.len(), HEADER_BYTES);
    assert_eq!(
        bytes,
        [
            1, 0, // version, flags
            23, 0, // tag = ParamSet, little-endian
            3, 0, // epoch
            0, 0, // reserved
            4, 3, 2, 1, // len
            0xEF, 0xBE, 0xAD, 0xDE, // seq
        ]
    );
    assert_eq!(Header::decode(&bytes, 3), Ok(h));
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
    // Every ControlMsg variant maps to a tag that is in ALL (the enumeration of the contract).
    let samples: Vec<ControlMsg> = vec![
        ControlMsg::Reset,
        ControlMsg::SetTransport {
            tempo_bpm: 120.0,
            numerator: 4,
            denominator: 4,
            launch_quantize: 3840,
        },
        ControlMsg::TrackCreate {
            track: TrackSlot(0),
            kind: TrackKind::Midi,
        },
        ControlMsg::TrackRemove {
            track: TrackSlot(0),
        },
        ControlMsg::TrackSetMix {
            track: TrackSlot(0),
            gain: 1.0,
            pan: 0.0,
            mute: false,
        },
        ControlMsg::TrackSetSolo {
            track: TrackSlot(0),
            solo: true,
        },
        ControlMsg::TrackArm {
            track: TrackSlot(0),
            armed: true,
        },
        ControlMsg::DeviceInsert {
            track: TrackSlot(0),
            device: engine::msg::DeviceSlot(0),
            kind: engine::msg::DeviceKind(1),
            state: vec![],
        },
        ControlMsg::DeviceRemove {
            track: TrackSlot(0),
            device: engine::msg::DeviceSlot(0),
        },
        ControlMsg::DeviceSetEnabled {
            track: TrackSlot(0),
            device: engine::msg::DeviceSlot(0),
            enabled: false,
        },
        ControlMsg::ParamSet {
            track: TrackSlot(0),
            device: engine::msg::DeviceSlot(0),
            param: engine::msg::ParamId(0),
            value: 0.5,
        },
        ControlMsg::ClipNotesUpload {
            clip: engine::msg::ClipSlot(0),
            length: 3840,
            looped: true,
            first: 0,
            total: 0,
            notes: vec![],
        },
        ControlMsg::ClipRemove {
            clip: engine::msg::ClipSlot(0),
        },
        ControlMsg::AudioSourceDeclare {
            source: engine::msg::SourceSlot(0),
            sample_rate: 48_000,
            channels: 2,
            frames: 0,
        },
        ControlMsg::AudioSourceRelease {
            source: engine::msg::SourceSlot(0),
        },
        ControlMsg::ClipAudioSet {
            clip: engine::msg::ClipSlot(0),
            source: engine::msg::SourceSlot(0),
            start: 0,
            length: 0,
        },
        ControlMsg::ClipPlace {
            track: TrackSlot(0),
            placement: engine::msg::PlacementSlot(0),
            clip: engine::msg::ClipSlot(0),
            start: 0,
            offset: 0,
            length: 3840,
        },
        ControlMsg::ClipUnplace {
            placement: engine::msg::PlacementSlot(0),
        },
        ControlMsg::AutomationLaneUpload {
            lane: engine::msg::LaneSlot(0),
            track: TrackSlot(0),
            target: engine::msg::AutomationTarget::MixerGain,
            first: 0,
            total: 0,
            points: vec![],
        },
        ControlMsg::AutomationLaneRemove {
            lane: engine::msg::LaneSlot(0),
        },
        ControlMsg::SceneSetSlot {
            track: TrackSlot(0),
            scene: engine::msg::SceneSlot(0),
            clip: None,
        },
        ControlMsg::LaunchClip {
            track: TrackSlot(0),
            scene: engine::msg::SceneSlot(0),
        },
        ControlMsg::LaunchScene {
            scene: engine::msg::SceneSlot(0),
        },
        ControlMsg::StopTrack {
            track: TrackSlot(0),
        },
        ControlMsg::StopAll,
        ControlMsg::Play { from: 0 },
        ControlMsg::Stop,
        ControlMsg::Seek { tick: 0 },
        ControlMsg::SetLoop { region: None },
        ControlMsg::SetRecording { recording: true },
        ControlMsg::NoteOn {
            track: TrackSlot(0),
            pitch: 60,
            velocity: 100,
        },
        ControlMsg::NoteOff {
            track: TrackSlot(0),
            pitch: 60,
        },
        ControlMsg::AllNotesOff {
            track: TrackSlot(0),
        },
    ];
    assert_eq!(samples.len(), Tag::ALL.len(), "one sample per variant");
    let mut seen: Vec<Tag> = samples.iter().map(ControlMsg::tag).collect();
    seen.sort_by_key(|t| *t as u16);
    let mut all = Tag::ALL.to_vec();
    all.sort_by_key(|t| *t as u16);
    assert_eq!(seen, all);
    for sample in &samples {
        assert_eq!(sample.check_limits(), Ok(()), "{sample:?} is within the limits");
        assert_eq!(
            sample.tag().is_live(),
            matches!(
                sample,
                ControlMsg::NoteOn { .. } | ControlMsg::NoteOff { .. } | ControlMsg::AllNotesOff { .. }
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
    assert_eq!(Header::decode(&[], 3), Err(MsgError::Truncated { needed: 16, available: 0 }));

    let mut version = good;
    version[0] = 2;
    assert_eq!(Header::decode(&version, 3), Err(MsgError::UnsupportedVersion(2)));

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
    // A stale epoch is reported before an unknown tag: the message is dropped either way.
    assert_eq!(
        Header::decode(&unknown, 4),
        Err(MsgError::StaleEpoch {
            message: 3,
            current: 4
        })
    );
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
    // The worst case: every table at its limit, each one doubled (the next table exists
    // next to the old one during a growth), the track buffers, the PCM budget, the rings.
    let notes = u64::from(MAX_NOTES) * size_of::<Note>() as u64;
    let points = u64::from(MAX_AUTOMATION_POINTS) * size_of::<engine::msg::AutomationPoint>() as u64;
    let placements = u64::from(MAX_PLACEMENTS) * 64;
    let clips = u64::from(MAX_CLIPS) * 64;
    let lanes = u64::from(MAX_LANES) * 64;
    let sources = u64::from(MAX_AUDIO_SOURCES) * 256;
    // Per track: its mix state, two stereo block buffers for the chain, the grid slots.
    let per_track = 256 + 4 * (MAX_BLOCK_FRAMES as u64) * 4 + u64::from(MAX_SCENES) * 8;
    let tracks = u64::from(MAX_TRACKS) * per_track;
    let devices = u64::from(MAX_TRACKS) * u64::from(MAX_DEVICES_PER_TRACK) * 4096;
    let tables = notes + points + placements + clips + lanes + sources + tracks;
    let rings = 2 * (RT_RING_CAPACITY as u64) * size_of::<RtMsg>() as u64
        + (LIVE_QUEUE_CAPACITY as u64) * size_of::<RtMsg>() as u64;
    let inbox = u64::from(MAX_CONTROL_MSG_BYTES) * 2;
    let feedback = Feedback::size_for(MAX_TRACKS as usize, (MAX_TRACKS * MAX_SCENES) as usize) as u64;
    let total = 2 * tables + devices + PCM_BUDGET_BYTES + rings + inbox + feedback;
    eprintln!("worst case: {} MiB", total >> 20);
    assert!(
        total < WASM_MAX_BYTES * 3 / 4,
        "{} MiB leaves no margin under 4 GiB",
        total >> 20
    );
    assert_eq!(
        u64::from(WASM_INITIAL_PAGES) * 65_536,
        16 << 20,
        "the initial memory is 16 MiB"
    );
}

#[test]
fn an_rt_message_is_small_and_sent_by_move() {
    // Moved through a ring slot: small enough that a slot copy is a few words.
    assert!(size_of::<RtMsg>() <= 64, "RtMsg is {} bytes", size_of::<RtMsg>());
    assert!(size_of::<Note>() <= 24, "Note is {} bytes", size_of::<Note>());
    assert_eq!(Feedback::size_for(0, 0), FEEDBACK_HEADER_BYTES);
    assert_eq!(Feedback::size_for(2, 3), (FEEDBACK_HEADER_BYTES + 32 + 3).next_multiple_of(4));
}
