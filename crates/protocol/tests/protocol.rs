use project::{DeviceId, TrackId};
use protocol::{
    AgentResult, Capabilities, ClientMessage, HelperUpdate, MidiInput, PROTOCOL_VERSION,
    ServerMessage, schema_bundle, single_synth,
};

#[test]
fn messages_round_trip_through_json() {
    let client = [
        ClientMessage::Pair {
            code: "12345678".into(),
        },
        ClientMessage::MidiListInputs,
        ClientMessage::MidiSelectInput { id: None },
        ClientMessage::MidiSelectInput {
            id: Some("a".into()),
        },
        ClientMessage::LiveRouting {
            epoch: 3,
            tracks: vec![0, 2],
        },
        ClientMessage::AgentResult {
            id: 1,
            result: AgentResult::Applied {
                output: serde_json::json!({ "track": "t1" }),
            },
        },
        ClientMessage::AgentResult {
            id: 2,
            result: AgentResult::Applied {
                output: serde_json::Value::Null,
            },
        },
        ClientMessage::AgentResult {
            id: 3,
            result: AgentResult::Refused {
                error: serde_json::json!({ "code": "not_found", "entity": "track", "id": "x" }),
            },
        },
        ClientMessage::AgentResult {
            id: 4,
            result: AgentResult::Invalid {
                message: "unknown variant".into(),
            },
        },
        ClientMessage::AgentResult {
            id: u64::MAX,
            result: AgentResult::Unavailable {
                reason: "no project".into(),
            },
        },
    ];
    // The page's spelling (AG-1).
    assert_eq!(
        serde_json::from_str::<ClientMessage>(
            r#"{"type":"agent.result","id":5,"result":{"status":"applied","output":{}}}"#
        )
        .unwrap(),
        ClientMessage::AgentResult {
            id: 5,
            result: AgentResult::Applied {
                output: serde_json::json!({})
            }
        }
    );
    // The page's spelling (#598).
    assert_eq!(
        serde_json::from_str::<ClientMessage>(r#"{"type":"live.routing","epoch":3,"tracks":[]}"#)
            .unwrap(),
        ClientMessage::LiveRouting {
            epoch: 3,
            tracks: vec![]
        }
    );
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
            audio_output: false,
        },
        ServerMessage::Paired { token: "t".into() },
        ServerMessage::AgentCall {
            id: 9,
            agent: "claude-code".into(),
            command: serde_json::json!({ "type": "track.create", "kind": "midi" }),
        },
        ServerMessage::PairingRefused { attempts_left: 2 },
        ServerMessage::Hello {
            protocol: 1,
            sample_rate: 48_000.0,
            buffer_frames: None,
            estimated_latency_ms: None,
            epoch: 7,
            capabilities: Capabilities {
                midi: true,
                disk: false,
            },
        },
        ServerMessage::StreamRefused {
            code: -4,
            at: 16,
            epoch: 7,
        },
        ServerMessage::MidiInputs {
            inputs: vec![MidiInput {
                id: "a".into(),
                name: "Keys".into(),
            }],
            selected: None,
        },
        ServerMessage::MidiMessage {
            data: vec![0x90, 60, 100],
            played: true,
        },
        ServerMessage::MidiMessage {
            data: vec![0xB0, 1, 64],
            played: false,
        },
        ServerMessage::Error {
            code: "midi.connect".into(),
            message: "x".into(),
        },
    ];
    let updates = [
        HelperUpdate::Disabled { reason: "r".into() },
        HelperUpdate::Idle,
        HelperUpdate::Checking,
        HelperUpdate::UpToDate {
            version: "0.1.11".into(),
        },
        HelperUpdate::Downloading {
            version: "0.1.12".into(),
            percent: 40,
        },
        HelperUpdate::Ready {
            version: "0.1.12".into(),
        },
        HelperUpdate::Failed { error: "e".into() },
    ];
    let server =
        server.into_iter().chain(
            updates
                .into_iter()
                .map(|status| ServerMessage::UpdateStatus {
                    current: Some("0.1.11".into()),
                    checked_at: Some(1),
                    status,
                }),
        );
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
        10,
        "add the new message to messages_round_trip_through_json"
    );
    let clients = bundle["client_message"]["oneOf"].as_array().unwrap();
    assert_eq!(
        clients.len(),
        5,
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
fn bundle_carries_the_command_contract() {
    let bundle = schema_bundle();
    let command = &bundle["command"];
    assert_eq!(command["input"], commands::schema());
    assert_eq!(command["output"], commands::output_schema());
    let catalog = command["catalog"].as_array().unwrap();
    assert_eq!(catalog.len(), commands::catalog().len());
    assert_eq!(catalog[0]["name"], "project.rename");
    assert!(bundle.get("command_v2").is_none());
}

#[test]
fn the_single_synth_ids_are_valid_and_distinct() {
    let bundle = schema_bundle();
    let track = TrackId::parse(single_synth::TRACK).unwrap();
    let device = DeviceId::parse(single_synth::DEVICE).unwrap();
    assert_ne!(track.0, device.0);
    assert_eq!(bundle["single_synth"]["track"], single_synth::TRACK);
    assert_eq!(bundle["single_synth"]["device"], single_synth::DEVICE);
}

#[test]
fn the_single_synth_kind_names_an_exported_descriptor() {
    let bundle = schema_bundle();
    let kind = &bundle["single_synth"]["kind"];
    assert_eq!(kind, "faulk.osc_synth");
    let devices = bundle["devices"].as_array().unwrap();
    assert_eq!(devices.iter().filter(|d| &d["kind"] == kind).count(), 1);
    // One exported descriptor per device of the registry, in its order.
    let kinds: Vec<_> = devices.iter().map(|d| d["kind"].clone()).collect();
    let registry: Vec<_> = engine::devices::REGISTRY
        .iter()
        .map(|b| serde_json::Value::from(b.descriptor.kind))
        .collect();
    assert_eq!(kinds, registry);
}

#[test]
fn a_command_cannot_be_smuggled_into_pairing() {
    let json = r#"{"type":"pair","code":"1","command":{"type":"live.all_notes_off"}}"#;
    assert!(serde_json::from_str::<ClientMessage>(json).is_err());
}

#[test]
fn bundle_exports_the_device_descriptors() {
    let bundle = schema_bundle();
    let devices = bundle["devices"].as_array().unwrap();
    let synth = devices
        .iter()
        .find(|d| d["kind"] == "faulk.osc_synth")
        .expect("the current synth is exported");
    assert_eq!(synth["name"], "Oscillator");
    assert_eq!(synth["role"], "instrument");
    assert_eq!(synth["kind_id"], 1);
    assert!(synth["midi_output"].is_null());
    let params = synth["params"].as_array().unwrap();
    // The same ids and defaults as the v1 export the web app reads today.
    assert_eq!(params.len(), bundle["param_ids"].as_object().unwrap().len());
    for p in params {
        let key = p["key"].as_str().unwrap();
        assert_eq!(p["id"], bundle["param_ids"][key], "id of {key}");
        assert_eq!(
            p["default"], bundle["param_defaults"][key],
            "default of {key}"
        );
        assert!(p["step"].is_number() && p["smoothing_seconds"].is_number());
    }
    assert_eq!(params[0]["values"][1], "saw");
    assert_eq!(params[5]["smoothing_seconds"], 0.01);
    assert_eq!(params[1]["unit"], "seconds");
    assert_eq!(params[1]["scale"], "log");
    // The test devices are not built-in devices.
    assert!(
        devices
            .iter()
            .all(|d| !d["kind"].as_str().unwrap().contains("test"))
    );
    // The descriptor's schema describes what is exported.
    let schema = &bundle["device_descriptor"];
    for field in [
        "kind",
        "kind_id",
        "version",
        "name",
        "role",
        "memory_bytes",
        "midi_output",
        "params",
    ] {
        assert!(schema["properties"][field].is_object(), "schema of {field}");
    }
    let param = &schema["$defs"]["ParamDescriptor"]["properties"];
    for field in [
        "id",
        "key",
        "description",
        "unit",
        "min",
        "max",
        "default",
        "scale",
        "step",
        "smoothing_seconds",
        "values",
    ] {
        assert!(param[field].is_object(), "schema of param {field}");
    }
}

#[test]
fn bundle_carries_the_faders_law_with_the_commands_bounds() {
    let bundle = schema_bundle();
    let taper = bundle["fader_taper"].as_array().unwrap();
    assert_eq!(taper.len(), engine::fader::TAPER.len());
    // The ends of the taper are the bounds the commands accept for a gain.
    assert_eq!(taper[0][1], project::MIN_GAIN_DB);
    assert_eq!(taper[taper.len() - 1][1], project::MAX_GAIN_DB);
    assert_eq!(engine::fader::MIN_DB, project::MIN_GAIN_DB);
    assert_eq!(engine::fader::MAX_DB, project::MAX_GAIN_DB);
    assert_eq!(taper[0][0], 0.0);
    assert_eq!(taper[taper.len() - 1][0], 1.0);
}
