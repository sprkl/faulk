//! Decoding any bytes never panics (MO-1): arbitrary payloads for every tag, arbitrary
//! streams and valid streams with corrupted bytes through the whole prepare side, then a
//! block of the audio side. A decoded message re-encodes to the same message.

use engine::device::DeviceSetup;
use engine::device::testing::{GAIN, TONE, factory};
use engine::engine::channel;
use engine::msg::{
    ClipSlot, ControlMsg, DeviceSlot, HEADER_BYTES, Header, MSG_VERSION, Note, SceneSlot, Tag,
    TrackKind, TrackSlot, encode_stream,
};
use engine::time::Tick;
use proptest::prelude::*;

fn valid_stream() -> Vec<u8> {
    let mut msgs = vec![
        ControlMsg::Reset { epoch: 1 },
        ControlMsg::TrackCreate {
            track: TrackSlot(0),
            kind: TrackKind::Midi,
        },
        ControlMsg::DeviceInsert {
            track: TrackSlot(0),
            device: DeviceSlot(0),
            kind: TONE.kind_id,
            state: vec![],
        },
        ControlMsg::DeviceInsert {
            track: TrackSlot(0),
            device: DeviceSlot(5),
            kind: GAIN.kind_id,
            state: engine::msg::encode_state(&[(engine::msg::ParamId(0), 0.5)]),
        },
        ControlMsg::SceneSetSlot {
            track: TrackSlot(0),
            scene: SceneSlot(30),
            clip: Some(ClipSlot(1)),
        },
        ControlMsg::NoteOn {
            track: TrackSlot(0),
            pitch: 60,
            velocity: 90,
        },
    ];
    msgs.push(ControlMsg::ClipNotesUpload {
        clip: ClipSlot(2000),
        length: Tick(960),
        looping: None,
        first: 0,
        total: 4,
        notes: vec![Note::default(); 4],
    });
    encode_stream(1, 0, &msgs).unwrap()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    #[test]
    fn any_payload_of_any_tag_decodes_or_is_refused(
        tag in prop::sample::select(Tag::ALL.to_vec()),
        payload in prop::collection::vec(any::<u8>(), 0..96),
    ) {
        let header = Header { version: MSG_VERSION, tag, epoch: 0, len: payload.len() as u32, seq: 0 };
        if let Ok(msg) = ControlMsg::decode(&header, &payload) {
            // What decodes re-encodes to a message that decodes to itself.
            let mut bytes = Vec::new();
            msg.encode(0, 0, &mut bytes).unwrap();
            let again = Header::decode(&bytes, if tag == Tag::Reset { msg_epoch(&msg).wrapping_sub(1) } else { 0 }).unwrap();
            let decoded = ControlMsg::decode(&again, &bytes[HEADER_BYTES..HEADER_BYTES + again.len as usize]);
            prop_assert_eq!(decoded, Ok(msg));
        }
    }

    #[test]
    fn any_bytes_through_the_prepare_side_never_panic(
        bytes in prop::collection::vec(any::<u8>(), 0..512),
    ) {
        let (mut p, mut e) = channel(DeviceSetup::default(), factory).unwrap();
        let _ = p.submit(&bytes);
        p.flush();
        e.begin_block();
        prop_assert_eq!(e.leaked(), 0);
    }

    #[test]
    fn a_valid_stream_with_corrupted_bytes_never_panics(
        flips in prop::collection::vec((any::<prop::sample::Index>(), any::<u8>()), 1..8),
        cut in any::<prop::sample::Index>(),
    ) {
        let mut stream = valid_stream();
        for (at, value) in &flips {
            let i = at.index(stream.len());
            stream[i] = *value;
        }
        let len = cut.index(stream.len() + 1);
        let (mut p, mut e) = channel(DeviceSetup::default(), factory).unwrap();
        let _ = p.submit(&stream[..len]);
        p.flush();
        e.begin_block();
        e.begin_block();
        prop_assert_eq!(e.leaked(), 0);
    }

    #[test]
    fn headers_of_any_bytes_never_panic(
        bytes in prop::collection::vec(any::<u8>(), 0..64),
        epoch in any::<u16>(),
    ) {
        let _ = Header::decode(&bytes, epoch);
    }
}

fn msg_epoch(msg: &ControlMsg) -> u16 {
    match msg {
        ControlMsg::Reset { epoch } => *epoch,
        _ => 0,
    }
}

#[test]
fn the_valid_stream_is_accepted_whole() {
    let (mut p, mut e) = channel(DeviceSetup::default(), factory).unwrap();
    p.submit(&valid_stream()).unwrap();
    e.begin_block();
    assert_eq!(p.rejected(), 0);
    assert_eq!(e.skipped(), 0);
    assert!(e.state().clip(ClipSlot(2000)).is_some());
    assert_eq!(e.live_events().len(), 1);
}
