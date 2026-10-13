use commands::{Applied, Command, SynthParam, UndoAction, UndoKind, param_ids, schema};
use engine::Synth;

/// One example of every command. `every_variant_has_an_example` keeps it complete.
fn examples() -> Vec<Command> {
    vec![
        Command::MidiListInputs,
        Command::MidiSelectInput {
            id: Some("in-1".into()),
        },
        Command::MidiSelectInput { id: None },
        Command::SynthSetParam {
            param: SynthParam::Attack,
            value: 0.25,
        },
        Command::NoteOn {
            note: 60,
            velocity: 100,
        },
        Command::NoteOff { note: 60 },
        Command::NoteAllOff,
    ]
}

fn schema_variants() -> Vec<(String, Option<String>)> {
    let schema = schema();
    let variants = schema["oneOf"].as_array().expect("Command is a oneOf");
    variants
        .iter()
        .map(|v| {
            let name = v["properties"]["type"]["const"]
                .as_str()
                .expect("each variant has a const type")
                .to_owned();
            let description = v["description"].as_str().map(str::to_owned);
            (name, description)
        })
        .collect()
}

#[test]
fn every_variant_has_an_example() {
    let mut names: Vec<_> = schema_variants().into_iter().map(|(n, _)| n).collect();
    let mut covered: Vec<_> = examples().iter().map(|c| c.name().to_owned()).collect();
    names.sort();
    covered.sort();
    covered.dedup();
    assert_eq!(names, covered);
}

#[test]
fn every_command_round_trips_through_json() {
    for command in examples() {
        let json = serde_json::to_string(&command).unwrap();
        let back: Command = serde_json::from_str(&json).unwrap();
        assert_eq!(back, command, "{json}");
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["type"], command.name());
    }
}

#[test]
fn every_command_has_a_description_for_agents() {
    for (name, description) in schema_variants() {
        let description = description.unwrap_or_default();
        assert!(description.len() > 20, "{name} lacks a description");
    }
}

#[test]
fn every_synth_param_has_a_description() {
    let schema = schema();
    let params = schema["$defs"]["SynthParam"]["oneOf"]
        .as_array()
        .expect("SynthParam is a oneOf of documented constants");
    assert_eq!(params.len(), SynthParam::ALL.len());
    for p in params {
        assert!(
            p["description"].as_str().is_some_and(|d| !d.is_empty()),
            "{p}"
        );
    }
}

#[test]
fn unknown_commands_and_fields_are_rejected() {
    assert!(serde_json::from_str::<Command>(r#"{"type":"track.delete"}"#).is_err());
    assert!(serde_json::from_str::<Command>(r#"{"type":"note.off","note":60,"x":1}"#).is_err());
    assert!(
        serde_json::from_str::<Command>(r#"{"type":"note.on","note":300,"velocity":1}"#).is_err()
    );
}

#[test]
fn set_param_undo_restores_every_parameter() {
    for param in SynthParam::ALL {
        let mut synth = Synth::new(48_000.0);
        let before = synth.param(param.to_engine());
        let applied = Command::SynthSetParam { param, value: 0.5 }.apply(&mut synth);
        let Applied::Done {
            undo: Some(undo @ UndoAction::SetParam { .. }),
        } = applied
        else {
            panic!("{param:?}: set_param must be undoable");
        };
        undo.into_command().apply(&mut synth);
        assert_eq!(synth.param(param.to_engine()), before, "{param:?}");
    }
}

#[test]
fn notes_are_not_undoable_and_midi_commands_are_for_the_host() {
    let mut synth = Synth::new(48_000.0);
    let on = Command::NoteOn {
        note: 60,
        velocity: 100,
    };
    assert_eq!(on.apply(&mut synth), Applied::Done { undo: None });
    assert_eq!(synth.active_voices(), 1);
    assert_eq!(
        Command::MidiListInputs.apply(&mut synth),
        Applied::NotForSynth
    );
    assert_eq!(
        Command::MidiSelectInput { id: None }.apply(&mut synth),
        Applied::NotForSynth
    );
}

#[test]
fn every_command_declares_its_undo_in_the_schema() {
    let schema = schema();
    let variants = schema["oneOf"].as_array().unwrap();
    for command in examples() {
        let variant = variants
            .iter()
            .find(|v| v["properties"]["type"]["const"] == command.name())
            .unwrap();
        let declared = variant["x-undo"].as_str().unwrap_or("missing");
        let expected = match command.undo_kind() {
            UndoKind::Inverse => "inverse",
            UndoKind::Host => "host",
            UndoKind::None => "none",
        };
        assert_eq!(declared, expected, "{}", command.name());
    }
}

#[test]
fn param_ids_match_the_serde_names_and_are_unique() {
    let ids = param_ids();
    assert_eq!(ids.len(), SynthParam::ALL.len());
    for (param, (name, id)) in SynthParam::ALL.iter().zip(&ids) {
        let json = serde_json::to_value(param).unwrap();
        assert_eq!(json, *name);
        assert_eq!(engine::Param::from_id(*id), Some(param.to_engine()));
    }
    let mut unique: Vec<_> = ids.iter().map(|(_, id)| id).collect();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), ids.len());
}

#[test]
fn all_off_releases_every_note() {
    let mut synth = Synth::new(48_000.0);
    for note in [60, 64, 67] {
        Command::NoteOn {
            note,
            velocity: 100,
        }
        .apply(&mut synth);
    }
    Command::NoteAllOff.apply(&mut synth);
    let mut l = vec![0.0; 48_000];
    let mut r = vec![0.0; 48_000];
    synth.render(&mut l, &mut r);
    assert_eq!(synth.active_voices(), 0);
}
