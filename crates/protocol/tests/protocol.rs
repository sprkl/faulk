use commands::Command;
use protocol::{ClientMessage, MidiInput, PROTOCOL_VERSION, ServerMessage, schema_bundle};

#[test]
fn messages_round_trip_through_json() {
    let client = [
        ClientMessage::Pair {
            code: "12345678".into(),
        },
        ClientMessage::Command {
            command: Command::NoteOn {
                note: 60,
                velocity: 1,
            },
        },
    ];
    for m in client {
        let json = serde_json::to_string(&m).unwrap();
        assert_eq!(
            serde_json::from_str::<ClientMessage>(&json).unwrap(),
            m,
            "{json}"
        );
    }
    let server = [
        ServerMessage::Probe {
            helper: "faulk-helper".into(),
            protocol: PROTOCOL_VERSION,
        },
        ServerMessage::Paired { token: "t".into() },
        ServerMessage::PairingRefused { attempts_left: 2 },
        ServerMessage::Hello {
            protocol: 1,
            sample_rate: 48_000.0,
            buffer_frames: None,
            estimated_latency_ms: None,
        },
        ServerMessage::MidiInputs {
            inputs: vec![MidiInput {
                id: "a".into(),
                name: "Keys".into(),
            }],
            selected: None,
        },
        ServerMessage::MidiActivity {
            note: 60,
            velocity: 0,
        },
        ServerMessage::Meter {
            peak: 0.5,
            voices: 2,
        },
        ServerMessage::Error {
            code: "midi.connect".into(),
            message: "x".into(),
        },
    ];
    for m in server {
        let json = serde_json::to_string(&m).unwrap();
        assert_eq!(
            serde_json::from_str::<ServerMessage>(&json).unwrap(),
            m,
            "{json}"
        );
    }
}

#[test]
fn every_server_message_is_in_the_round_trip_test() {
    let bundle = schema_bundle();
    let variants = bundle["server_message"]["oneOf"].as_array().unwrap();
    assert_eq!(
        variants.len(),
        8,
        "add the new message to messages_round_trip_through_json"
    );
    let clients = bundle["client_message"]["oneOf"].as_array().unwrap();
    assert_eq!(
        clients.len(),
        2,
        "add the new message to messages_round_trip_through_json"
    );
}

#[test]
fn bundle_carries_version_and_param_ids() {
    let bundle = schema_bundle();
    assert_eq!(bundle["protocol_version"], PROTOCOL_VERSION);
    assert_eq!(bundle["param_ids"]["waveform"], 0);
    assert_eq!(bundle["param_ids"].as_object().unwrap().len(), 6);
}

#[test]
fn a_command_cannot_be_smuggled_into_pairing() {
    let json = r#"{"type":"pair","code":"1","command":{"type":"note.all_off"}}"#;
    assert!(serde_json::from_str::<ClientMessage>(json).is_err());
}
