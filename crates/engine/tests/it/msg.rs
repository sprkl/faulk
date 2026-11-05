//! The frozen parts of `engine::msg` (CT-3): header codec, tags, limits, sizes.

use engine::clip_player::PackedNote;
use engine::engine::{Clip, DeviceEntry, Lane, Source, Track};
use engine::msg::{
    ACTIVE_CLIP_BYTES, ActiveClip, AutomationPoint, AutomationTarget, ClipSlot, ControlMsg,
    DeviceKind, DeviceSlot, FEEDBACK_HEADER_BYTES, Feedback, HEADER_BYTES, Header, LaneSlot,
    MSG_ALIGN, MSG_VERSION, MsgError, Note, ParamId, Placement, RECORDED_NOTE_WIRE_BYTES,
    RecordedNote, RtMsg, SceneSlot, SourceSlot, TRACK_METER_BYTES, Tag, TrackKind, TrackMeter,
    TrackSlot, limits,
};
use engine::msg::{Garbage, LiveEvent};
use engine::time::{Frame, LoopRegion, Quantize, Tempo, Tick, TimeSignature};

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
    // The whole message: the header followed by `len` bytes of payload.
    let mut whole = bytes.to_vec();
    whole.resize(HEADER_BYTES + h.len as usize, 0);
    assert_eq!(Header::decode(&whole, 3), Ok(h));
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
            pan: 0.0,
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
        ControlMsg::HoldMix {
            track: t,
            gain: Some(1.0),
            pan: None,
        },
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
            looping: None,
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
            media: [0; 32],
        },
        ControlMsg::AudioSourceRelease {
            source: SourceSlot(0),
        },
        ControlMsg::ClipAudioSet {
            clip: ClipSlot(0),
            source: SourceSlot(0),
            start: Frame(0),
            length: Frame(0),
            gain: 1.0,
            looping: None,
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
        ControlMsg::AllSoundOff,
        ControlMsg::BackToArrangement,
        ControlMsg::BackTrackToArrangement { track: t },
        ControlMsg::Play { from: Tick(0) },
        ControlMsg::Stop,
        ControlMsg::Seek { tick: Tick(0) },
        ControlMsg::SetLoop { region: None },
        ControlMsg::SetRecording {
            recording: true,
            shift_us: -42_000,
        },
        ControlMsg::NoteOn {
            track: t,
            pitch: 60,
            velocity: 100,
            at: None,
        },
        ControlMsg::NoteOff {
            track: t,
            pitch: 60,
            at: None,
        },
        ControlMsg::AllNotesOff { track: t },
    ]
}

#[test]
fn every_tag_round_trips_and_every_variant_has_its_tag() {
    for tag in Tag::ALL {
        assert_eq!(Tag::from_u16(tag as u16), Some(tag));
        let bytes = header(tag, 0).encode();
        // A Reset is read against an older epoch (it brings a newer one).
        let current = if tag == Tag::Reset { 2 } else { 3 };
        assert_eq!(Header::decode(&bytes, current).map(|h| h.tag), Ok(tag));
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
    let mut good = header(Tag::Stop, 4).encode().to_vec();
    good.extend_from_slice(&[0; 4]);
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

    let mut version = good.clone();
    version[0] = 2;
    assert_eq!(
        Header::decode(&version, 3),
        Err(MsgError::UnsupportedVersion(2))
    );

    let mut flags = good.clone();
    flags[1] = 1;
    assert_eq!(Header::decode(&flags, 3), Err(MsgError::Corrupt));
    let mut reserved = good.clone();
    reserved[7] = 0x80;
    assert_eq!(Header::decode(&reserved, 3), Err(MsgError::Corrupt));

    let mut large = good.clone();
    large[8..12].copy_from_slice(&(limits::MAX_CONTROL_MSG_BYTES + 1).to_le_bytes());
    assert_eq!(
        Header::decode(&large, 3),
        Err(MsgError::TooLarge(limits::MAX_CONTROL_MSG_BYTES + 1))
    );

    let mut unknown = good.clone();
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
    // Only a Reset crosses epochs, and only forward: a late Reset of a past epoch, or a
    // duplicate of the current one, is stale.
    let reset = header(Tag::Reset, 0).encode();
    assert_eq!(Header::decode(&reset, 2).map(|h| h.epoch), Ok(3));
    for current in [3, 4] {
        assert_eq!(
            Header::decode(&reset, current),
            Err(MsgError::StaleEpoch {
                message: 3,
                current
            })
        );
    }
    // A payload shorter than `len` announces is truncated, whatever the header says.
    let short = header(Tag::Stop, 4).encode();
    assert_eq!(
        Header::decode(&short, 3),
        Err(MsgError::Truncated {
            needed: HEADER_BYTES + 4,
            available: HEADER_BYTES
        })
    );
    let mut whole = short.to_vec();
    whole.extend_from_slice(&[0; 4]);
    assert!(Header::decode(&whole, 3).is_ok());
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
    // A clip's notes as the clip player keeps them (CL-2): a packed note and its two
    // entries of the delivery order each, one held bit; per clip, its count and a held
    // word at most for the rounding.
    let notes = u64::from(MAX_NOTES) * (size_of::<PackedNote>() + 2 * size_of::<u32>()) as u64
        + u64::from(MAX_NOTES).div_ceil(32) * 4
        + u64::from(MAX_CLIPS) * 8;
    let points = u64::from(MAX_AUTOMATION_POINTS) * size_of::<AutomationPoint>() as u64;
    let placements = u64::from(MAX_PLACEMENTS) * size_of::<Placement>() as u64;
    // The runtime tables as MO-1 builds them: a slot (a pointer) per entry plus the boxed
    // entry; clips and lanes in pages (their directory is a pointer per page).
    let boxed = |n: u32, entry: usize| u64::from(n) * (size_of::<Option<Box<u8>>>() + entry) as u64;
    let clips = boxed(MAX_CLIPS, size_of::<Clip>());
    let lanes = boxed(MAX_LANES, size_of::<Lane>());
    let sources = boxed(MAX_AUDIO_SOURCES, size_of::<Source>());
    // Per track: its slot and box, two stereo block buffers for the chain (MO-3), its row
    // of device slots with their boxed entries (the devices themselves count against the
    // engine budget by their declared sizes), its row of grid slots.
    let per_track = (size_of::<Option<Box<Track>>>() + size_of::<Track>()) as u64
        + 4 * (MAX_BLOCK_FRAMES as u64) * 4
        + u64::from(MAX_DEVICES_PER_TRACK)
            * (size_of::<Option<Box<DeviceEntry>>>() + size_of::<DeviceEntry>()) as u64
        + u64::from(MAX_SCENES) * size_of::<Option<ClipSlot>>() as u64;
    let tracks = u64::from(MAX_TRACKS) * per_track;
    let largest = notes.max(points).max(placements);
    let tables = notes + points + placements + clips + lanes + sources + tracks;
    // The rt ring and the prepare side's queue, the garbage ring, the live queue and the
    // block's copy of it.
    let rings = (RT_RING_CAPACITY + PENDING_QUEUE_CAPACITY) as u64 * size_of::<RtMsg>() as u64
        + (GARBAGE_RING_CAPACITY as u64) * size_of::<Garbage>() as u64
        + 2 * (LIVE_QUEUE_CAPACITY as u64) * size_of::<LiveEvent>() as u64;
    let inbox = u64::from(MAX_CONTROL_MSG_BYTES) * 2;
    let feedback = Feedback::max_size_for(MAX_TRACKS as usize) as u64;
    // The graph's latency buffers (#749): one compensation ring and one metronome line for
    // the whole engine, whatever the number of tracks.
    let compensation = engine::graph::Graph::latency_bytes(MAX_BLOCK_FRAMES);
    assert!(
        compensation < 2 << 20,
        "{compensation} bytes of compensation"
    );
    let total = 2 * tables + largest + rings + inbox + feedback + compensation;
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
    assert_eq!(
        Feedback::size_for(0, 0, 0),
        FEEDBACK_HEADER_BYTES + TRACK_METER_BYTES,
        "the output's meter always follows the track meters"
    );
    // The wire sizes of the Feedback records are their repr(C) sizes.
    assert_eq!(size_of::<TrackMeter>(), TRACK_METER_BYTES);
    assert_eq!(size_of::<ActiveClip>(), ACTIVE_CLIP_BYTES);
    assert_eq!(size_of::<RecordedNote>(), RECORDED_NOTE_WIRE_BYTES);
    assert_eq!(
        Feedback::size_for(2, 3, 4),
        FEEDBACK_HEADER_BYTES
            + 3 * TRACK_METER_BYTES
            + 3 * ACTIVE_CLIP_BYTES
            + 4 * RECORDED_NOTE_WIRE_BYTES
    );
    // The largest report of the largest session stays small enough for a WebSocket frame.
    assert!(Feedback::max_size_for(limits::MAX_TRACKS as usize) < 256 << 10);
}

// ---------------------------------------------------------------- payload codecs (MO-1)

/// The samples with their lists filled, so that the round trip covers list items too.
fn rich_samples() -> Vec<ControlMsg> {
    use engine::msg::{Curve, Span};
    let mut out = samples();
    // Stamped live notes (#499): the stamp is 8 more bytes, read back exactly.
    out.push(ControlMsg::NoteOn {
        track: TrackSlot(3),
        pitch: 61,
        velocity: 90,
        at: Some(u64::MAX - 7),
    });
    out.push(ControlMsg::NoteOff {
        track: TrackSlot(3),
        pitch: 61,
        at: Some(0),
    });
    out.push(ControlMsg::ClipNotesUpload {
        clip: ClipSlot(8),
        length: Tick(7680),
        looping: LoopRegion::new(Tick(3840), Tick(7200)),
        first: 0,
        total: 1,
        notes: vec![Note {
            cents: 37,
            chance: 64,
            repeat: 3,
            ..Note::default()
        }],
    });
    out.push(ControlMsg::ClipNotesUpload {
        clip: ClipSlot(7),
        length: Tick(-5),
        looping: None,
        first: 3,
        total: 9,
        notes: vec![
            Note {
                start: Tick(i64::MIN),
                length: Tick(i64::MAX),
                pitch: 127,
                velocity: 0,
                cents: -50,
                chance: 0,
                repeat: 16,
            };
            3
        ],
    });
    out.push(ControlMsg::TrackPlacementsUpload {
        track: TrackSlot(u32::MAX),
        first: 0,
        total: 2,
        placements: vec![
            Placement {
                placement: engine::msg::PlacementSlot(4),
                clip: ClipSlot(5),
                start: Tick(960),
                span: Span::Frames {
                    offset: Frame(u64::MAX),
                    length: Frame(3),
                },
            },
            Placement::default(),
        ],
    });
    out.push(ControlMsg::AutomationLaneUpload {
        lane: LaneSlot(1),
        track: TrackSlot(2),
        target: AutomationTarget::DeviceParam {
            device: DeviceSlot(u16::MAX),
            param: ParamId(9),
        },
        first: 0,
        total: 1,
        points: vec![AutomationPoint {
            tick: Tick(1),
            value: -0.5,
            curve: Curve::Linear,
        }],
    });
    out.push(ControlMsg::DeviceInsert {
        track: TrackSlot(0),
        device: DeviceSlot(3),
        kind: DeviceKind(0xFFFF_0001),
        state: engine::msg::encode_state(&[(ParamId(0), 0.25), (ParamId(1), -1.0)]),
    });
    out.push(ControlMsg::SetLoop {
        region: engine::time::LoopRegion::new(Tick(-960), Tick(3840)),
    });
    out.push(ControlMsg::SelectTrack { track: None });
    out.push(ControlMsg::SceneSetSlot {
        track: TrackSlot(1),
        scene: SceneSlot(2),
        clip: Some(ClipSlot(3)),
    });
    out
}

#[test]
fn every_variant_round_trips_through_its_payload_codec() {
    let samples = rich_samples();
    let mut tags: Vec<u16> = samples.iter().map(|m| m.tag() as u16).collect();
    tags.sort_unstable();
    tags.dedup();
    assert_eq!(tags.len(), Tag::ALL.len(), "every variant is encoded");
    let mut stream = Vec::new();
    for (seq, msg) in samples.iter().enumerate() {
        let start = stream.len();
        msg.encode(5, seq as u32, &mut stream).unwrap();
        assert_eq!((stream.len() - start) % MSG_ALIGN, 0, "{msg:?} is padded");
        // A Reset is read against an older epoch.
        let current = if msg.tag() == Tag::Reset { 0 } else { 5 };
        let header = Header::decode(&stream[start..], current).unwrap();
        assert_eq!(header.seq, seq as u32);
        let payload = &stream[start + HEADER_BYTES..start + HEADER_BYTES + header.len as usize];
        assert_eq!(ControlMsg::decode(&header, payload).as_ref(), Ok(msg));
        // One byte more or less is a bad payload, never a panic nor a wrong message.
        let tag = msg.tag() as u16;
        if !payload.is_empty() {
            let short = Header {
                len: header.len - 1,
                ..header
            };
            assert_eq!(
                ControlMsg::decode(&short, &payload[..payload.len() - 1]),
                Err(MsgError::BadPayload { tag })
            );
        }
        let mut long = payload.to_vec();
        long.push(0);
        let longer = Header {
            len: header.len + 1,
            ..header
        };
        assert_eq!(
            ControlMsg::decode(&longer, &long),
            Err(MsgError::BadPayload { tag }),
            "{msg:?} with a trailing byte"
        );
    }
}

#[test]
fn out_of_range_values_are_bad_payloads() {
    let decode = |msg: &ControlMsg, at: usize, value: &[u8]| {
        let mut bytes = Vec::new();
        msg.encode(0, 0, &mut bytes).unwrap();
        let header =
            Header::decode(&bytes, if msg.tag() == Tag::Reset { 65535 } else { 0 }).unwrap();
        let mut payload = bytes[HEADER_BYTES..HEADER_BYTES + header.len as usize].to_vec();
        payload[at..at + value.len()].copy_from_slice(value);
        ControlMsg::decode(&header, &payload)
    };
    let bad = |tag: Tag| Err(MsgError::BadPayload { tag: tag as u16 });
    let solo = ControlMsg::TrackSetSolo {
        track: TrackSlot(0),
        solo: true,
    };
    assert_eq!(decode(&solo, 4, &[2]), bad(Tag::TrackSetSolo));
    let create = ControlMsg::TrackCreate {
        track: TrackSlot(0),
        kind: TrackKind::Midi,
    };
    assert_eq!(decode(&create, 4, &[2]), bad(Tag::TrackCreate));
    let master = ControlMsg::SetMaster {
        gain: 1.0,
        pan: 0.0,
        mute: false,
    };
    assert_eq!(
        decode(&master, 0, &f32::NAN.to_le_bytes()),
        bad(Tag::SetMaster)
    );
    assert_eq!(
        decode(&master, 0, &f32::INFINITY.to_le_bytes()),
        bad(Tag::SetMaster)
    );
    let transport = ControlMsg::SetTransport {
        tempo: Tempo::clamped(120.0),
        signature: TimeSignature::COMMON,
        launch_quantize: Quantize::Bar,
    };
    assert_eq!(
        decode(&transport, 0, &5.0f64.to_le_bytes()),
        bad(Tag::SetTransport),
        "tempo below 20"
    );
    assert_eq!(
        decode(&transport, 9, &[3]),
        bad(Tag::SetTransport),
        "3 is not a power of two"
    );
    assert_eq!(
        decode(&transport, 10, &[7]),
        bad(Tag::SetTransport),
        "no 8th quantize"
    );
    let on = ControlMsg::NoteOn {
        track: TrackSlot(0),
        pitch: 60,
        velocity: 1,
        at: None,
    };
    assert_eq!(decode(&on, 4, &[128]), bad(Tag::NoteOn));
    let looped = ControlMsg::SetLoop {
        region: engine::time::LoopRegion::new(Tick(0), Tick(1)),
    };
    assert_eq!(
        decode(&looped, 9, &0i64.to_le_bytes()),
        bad(Tag::SetLoop),
        "end <= start"
    );
    // A list whose count does not match the bytes left, before anything is allocated.
    let upload = ControlMsg::ClipNotesUpload {
        clip: ClipSlot(0),
        length: Tick(1),
        looping: None,
        first: 0,
        total: 1,
        notes: vec![Note::default()],
    };
    assert_eq!(
        decode(&upload, 37, &u32::MAX.to_le_bytes()),
        bad(Tag::ClipNotesUpload)
    );
    // A note's cents, chance and repeat outside their ranges (CL-2, #225): payload bytes
    // 59, 60 and 61 (clip 4, length 8, loop 17, first 4, total 4, count 4, then start 8,
    // length 8, pitch, velocity).
    for (at, value) in [
        (59, 51u8),
        (59, (-51i8) as u8),
        (60, 101),
        (61, 0),
        (61, 17),
    ] {
        assert_eq!(
            decode(&upload, at, &[value]),
            bad(Tag::ClipNotesUpload),
            "{at}"
        );
    }
    assert!(decode(&upload, 59, &[(-50i8) as u8]).is_ok());
    // A clip loop outside `[0, length]`, or empty.
    let looped = ControlMsg::ClipNotesUpload {
        clip: ClipSlot(0),
        length: Tick(100),
        looping: LoopRegion::new(Tick(0), Tick(100)),
        first: 0,
        total: 0,
        notes: vec![],
    };
    assert!(decode(&looped, 12, &[1]).is_ok());
    assert_eq!(
        decode(&looped, 21, &101i64.to_le_bytes()),
        bad(Tag::ClipNotesUpload)
    );
    assert_eq!(
        decode(&looped, 13, &(-1i64).to_le_bytes()),
        bad(Tag::ClipNotesUpload)
    );
    assert_eq!(
        decode(&looped, 13, &100i64.to_le_bytes()),
        bad(Tag::ClipNotesUpload)
    );
    // A device state that is not whole pairs, or holds a non-finite value (written by hand:
    // the encoder refuses both).
    let insert = Header {
        version: MSG_VERSION,
        tag: Tag::DeviceInsert,
        epoch: 0,
        len: 0,
        seq: 0,
    };
    let mut payload = vec![0u8; 10];
    payload.extend_from_slice(&[1, 2, 3]);
    let h = Header {
        len: payload.len() as u32,
        ..insert
    };
    assert_eq!(ControlMsg::decode(&h, &payload), bad(Tag::DeviceInsert));
    let mut payload = vec![0u8; 10];
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload.extend_from_slice(&f32::NAN.to_le_bytes());
    let h = Header {
        len: payload.len() as u32,
        ..insert
    };
    assert_eq!(ControlMsg::decode(&h, &payload), bad(Tag::DeviceInsert));
}

#[test]
fn a_payload_over_the_limit_is_refused_by_the_encoder() {
    let notes = (limits::MAX_CONTROL_MSG_BYTES as usize) / engine::msg::NOTE_WIRE_BYTES + 1;
    let msg = ControlMsg::ClipNotesUpload {
        clip: ClipSlot(0),
        length: Tick(1),
        looping: None,
        first: 0,
        total: notes as u32,
        notes: vec![Note::default(); notes],
    };
    let mut out = vec![9];
    assert!(matches!(
        msg.encode(0, 0, &mut out),
        Err(MsgError::TooLarge(_))
    ));
    assert_eq!(out, vec![9], "nothing appended");
}

#[test]
fn the_encoder_refuses_what_the_decoder_would() {
    let tag = |t: Tag| Err(MsgError::BadPayload { tag: t as u16 });
    for (msg, t) in [
        (
            ControlMsg::TrackSetMix {
                track: TrackSlot(0),
                gain: f32::NAN,
                pan: 0.0,
                mute: false,
            },
            Tag::TrackSetMix,
        ),
        (
            ControlMsg::HoldMix {
                track: TrackSlot(0),
                gain: Some(f32::INFINITY),
                pan: None,
            },
            Tag::HoldMix,
        ),
        (
            ControlMsg::NoteOn {
                track: TrackSlot(0),
                pitch: 200,
                velocity: 1,
                at: None,
            },
            Tag::NoteOn,
        ),
        (
            ControlMsg::DeviceInsert {
                track: TrackSlot(0),
                device: DeviceSlot(0),
                kind: DeviceKind(1),
                state: vec![1, 2, 3],
            },
            Tag::DeviceInsert,
        ),
        (
            ControlMsg::DeviceInsert {
                track: TrackSlot(0),
                device: DeviceSlot(0),
                kind: DeviceKind(1),
                state: engine::msg::encode_state(&[(ParamId(0), f32::INFINITY)]),
            },
            Tag::DeviceInsert,
        ),
    ] {
        let mut out = vec![7];
        assert_eq!(msg.encode(0, 0, &mut out), tag(t), "{msg:?}");
        assert_eq!(out, vec![7], "nothing appended");
    }
}

/// #499: a live note's stamp is written only when there is one, so an unstamped note is the
/// same bytes as before stamps existed (the recorded streams stay valid), and a stamped one
/// is exactly 8 bytes longer.
#[test]
fn a_live_notes_stamp_adds_eight_bytes_and_only_when_present() {
    let len = |msg: ControlMsg| {
        let mut bytes = Vec::new();
        msg.encode(0, 0, &mut bytes).unwrap();
        Header::decode(&bytes, 0).unwrap().len
    };
    let on = |at| ControlMsg::NoteOn {
        track: TrackSlot(1),
        pitch: 60,
        velocity: 100,
        at,
    };
    let off = |at| ControlMsg::NoteOff {
        track: TrackSlot(1),
        pitch: 60,
        at,
    };
    assert_eq!(len(on(None)), 6, "track, pitch, velocity: as before #499");
    assert_eq!(len(on(Some(48_000))), 14);
    assert_eq!(len(off(None)), 5);
    assert_eq!(len(off(Some(48_000))), 13);
}
