//! The command contract v2 (CT-4), checked on every declared command.
//!
//! The rule: every command has a description, an input and an output schema, and its
//! undo is tested. `every_command_has_an_example` refuses a command that no example
//! covers, and `examples` are checked by the harness of `commands::testing`.

use ::project::TrackId;
use commands::export::{ExportBitDepth, ExportRegion};
use commands::testing::{Fixture, check, check_refusal, context, fixture, state};
use commands::{
    Author, Command, CommandError, Kind, Meta, Output, ProjectEvent, State, UndoKind, catalog,
    device, history, launch, live, midi, mixer, project, view,
};

mod arrangement_examples;
mod automation_examples;
mod clip_examples;
mod device_examples;
mod grid_examples;
mod query_examples;
mod record_examples;
mod track_examples;
mod transport_examples;

const TRACK: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";

fn track() -> TrackId {
    TrackId::parse(TRACK).unwrap()
}

/// The fixture after `history.undo`: one entry to redo.
fn undone(f: &Fixture) -> State {
    let mut s = f.state.clone();
    commands::run(
        &Command::History(history::Command::Undo {}),
        &mut s,
        &mut context(),
    )
    .unwrap();
    s
}

/// One example of every command (at least), with the state it runs from.
/// `every_command_has_an_example` keeps it complete; the harness checks each.
fn examples() -> Vec<(Command, State)> {
    let f = fixture();
    let on = |c: Command| (c, f.state.clone());
    vec![
        on(Command::Project(project::Command::Rename {
            name: "  Demo song  ".into(),
        })),
        on(Command::Project(project::Command::Open {
            folder: commands::testing::OPENED_FOLDER.into(),
        })),
        on(Command::Project(project::Command::New { name: None })),
        on(Command::Project(project::Command::Save {})),
        on(Command::Project(project::Command::SaveAs {})),
        on(Command::Project(project::Command::ExportPack {})),
        on(Command::Project(project::Command::ImportPack {})),
        on(Command::Project(project::Command::OpenUrl {
            url: "https://cdn.jsdelivr.net/gh/sprkl/faulk-journey@main/tracks/2026-11-02-neon-sur-la-rocade/project.faulkpack".into(),
        })),
        on(Command::Project(project::Command::ListStored {})),
        on(Command::Project(project::Command::ReopenLast {})),
        on(Command::Project(project::Command::OpenFolder {})),
        on(Command::Project(project::Command::OpenRecent {
            id: "recent-1".into(),
        })),
        on(Command::Project(project::Command::OpenBrowserProject {
            dir: "3f2b8c1e-0a52-4c3d-9b7e-1d2f4a6b8c0e".into(),
        })),
        on(Command::Project(project::Command::DeleteBrowserProject {
            dir: "3f2b8c1e-0a52-4c3d-9b7e-1d2f4a6b8c0e".into(),
        })),
        on(Command::Project(project::Command::KeepMine {})),
        on(Command::Project(project::Command::LoadFolderVersion {})),
        on(Command::Project(project::Command::AllowAccess {})),
        (
            Command::Project(project::Command::ExportAudio {
                region: None,
                bit_depth: None,
                sample_rate: None,
                from_bar: None,
                to_bar: None,
            }),
            arrangement_examples::fx().state,
        ),
        (
            Command::Project(project::Command::ExportAudio {
                region: Some(ExportRegion::Bars),
                bit_depth: Some(ExportBitDepth::Pcm16),
                sample_rate: Some(44_100),
                from_bar: Some(2),
                to_bar: Some(3),
            }),
            arrangement_examples::fx().state,
        ),
        (
            Command::Project(project::Command::ExportAudio {
                region: Some(ExportRegion::Loop),
                bit_depth: Some(ExportBitDepth::Float32),
                sample_rate: Some(96_000),
                from_bar: None,
                to_bar: None,
            }),
            transport_examples::with_region(),
        ),
        on(Command::Device(device::Command::SetParam {
            device: f.device.clone(),
            param: "attack".into(),
            value: 0.25,
        })),
        on(Command::Midi(midi::Command::ListInputs {})),
        on(Command::Midi(midi::Command::SelectInput {
            id: Some("in-1".into()),
        })),
        on(Command::Midi(midi::Command::SelectInput { id: None })),
        on(Command::Live(live::Command::NoteOn {
            track: f.track.clone(),
            note: 60,
            velocity: 100,
        })),
        on(Command::Live(live::Command::NoteOn {
            track: f.track.clone(),
            note: 127,
            velocity: 0,
        })),
        on(Command::Live(live::Command::NoteOff {
            track: f.track.clone(),
            note: 60,
        })),
        on(Command::Live(live::Command::AllNotesOff {})),
        on(Command::Live(live::Command::AllSoundOff {})),
        on(Command::Launch(launch::Command::BackToArrangement {})),
        on(Command::Launch(launch::Command::BackTrackToArrangement {
            track: f.track.clone(),
        })),
        on(Command::Live(live::Command::InputNoteOn {
            note: 60,
            velocity: 100,
        })),
        on(Command::Live(live::Command::InputNoteOff { note: 60 })),
        on(Command::Live(live::Command::ControlChange {
            controller: 1,
            value: 64,
        })),
        on(Command::Mixer(mixer::Command::SetVolume {
            track: f.track.clone(),
            gain_db: Some(-6.0),
        })),
        on(Command::Mixer(mixer::Command::SetPan {
            track: f.track.clone(),
            pan: -0.5,
        })),
        on(Command::Mixer(mixer::Command::SetMute {
            track: f.track.clone(),
            mute: true,
        })),
        on(Command::Mixer(mixer::Command::SetMasterVolume {
            gain_db: Some(-3.0),
        })),
        on(Command::Mixer(mixer::Command::SetMasterPan { pan: -0.5 })),
        on(Command::Mixer(mixer::Command::SetSolo {
            track: f.track.clone(),
            solo: true,
        })),
        on(Command::History(history::Command::Undo {})),
        (Command::History(history::Command::Redo {}), undone(&f)),
        on(Command::History(history::Command::BeginGroup {})),
        (Command::History(history::Command::EndGroup {}), {
            let mut s = f.state.clone();
            let group = s.history.new_group();
            s.history.open_group = Some((group, Author::User));
            s
        }),
        on(Command::View(view::Command::SelectTrack {
            track: Some(f.track.clone()),
        })),
        on(Command::View(view::Command::SelectTrack { track: None })),
    ]
    .into_iter()
    .chain(clip_examples::examples())
    .chain(arrangement_examples::examples())
    .chain(grid_examples::examples())
    .chain(transport_examples::examples())
    .chain(track_examples::examples())
    .chain(device_examples::examples())
    .chain(automation_examples::examples())
    .chain(query_examples::examples())
    .chain(record_examples::examples())
    .collect()
}

#[test]
fn every_example_meets_the_contract() {
    for (command, state) in examples() {
        check(&command, &state, &context()).unwrap();
    }
}

#[test]
fn the_fixture_is_a_valid_project() {
    let f = fixture();
    f.state.project.validate().unwrap();
    assert_eq!(f.state.project.name, "Demo");
    assert_eq!(f.state.history.undo.len(), 1);
}

#[test]
fn every_command_has_an_example() {
    // The declared commands come from the macro (`METAS`), the examples from this file:
    // two independent lists.
    let mut declared: Vec<_> = Command::metas().iter().map(|m| m.name).collect();
    let mut covered: Vec<_> = examples().iter().map(|(c, _)| c.name()).collect();
    declared.sort();
    covered.sort();
    covered.dedup();
    assert_eq!(
        declared, covered,
        "a command without an example, or an example of nothing"
    );
}

#[test]
fn the_sixteen_domains_are_frozen_in_the_spec_order() {
    assert_eq!(
        Command::DOMAINS,
        [
            "project",
            "track",
            "device",
            "clip",
            "note",
            "scene",
            "launch",
            "arrangement",
            "automation",
            "transport",
            "mixer",
            "record",
            "midi",
            "live",
            "history",
            "view",
        ]
    );
}

#[test]
fn every_name_is_domain_dot_verb_in_its_own_domain() {
    let mut seen = Vec::new();
    for meta in Command::metas() {
        let (domain, verb) = meta
            .name
            .split_once('.')
            .unwrap_or_else(|| panic!("{} is not domain.verb", meta.name));
        assert!(
            Command::DOMAINS.contains(&domain),
            "{}: unknown domain",
            meta.name
        );
        assert!(!verb.is_empty(), "{}: empty verb", meta.name);
        assert!(
            verb.bytes()
                .all(|b| b.is_ascii_lowercase() || b == b'_' || b == b'.'),
            "{}: a verb is snake_case",
            meta.name
        );
        assert!(!seen.contains(&meta.name), "{} declared twice", meta.name);
        seen.push(meta.name);
    }
}

#[test]
fn each_metadata_rule_refuses_its_case() {
    let meta = |kind, undo, destructive| Meta {
        name: "x.y",
        kind,
        undo,
        destructive,
    };
    let refused = |m: Meta, expected: &str| {
        let err = m.check().unwrap_err();
        assert!(err.contains(expected), "{m:?}: {err}");
    };
    refused(
        meta(Kind::Live, UndoKind::Inverse, false),
        "cannot carry an inverse",
    );
    refused(
        meta(Kind::Query, UndoKind::Inverse, true),
        "cannot carry an inverse",
    );
    refused(
        meta(Kind::Live, UndoKind::None, true),
        "only an edit can be destructive",
    );
    refused(
        meta(Kind::Query, UndoKind::None, true),
        "only an edit can be destructive",
    );
    refused(
        meta(Kind::Edit, UndoKind::None, false),
        "must be declared destructive",
    );
    refused(
        meta(Kind::Edit, UndoKind::Inverse, true),
        "is not destructive",
    );
    for ok in [
        meta(Kind::Edit, UndoKind::Inverse, false),
        meta(Kind::Edit, UndoKind::None, true),
        meta(Kind::Live, UndoKind::None, false),
        meta(Kind::Query, UndoKind::None, false),
    ] {
        ok.check().unwrap();
    }
}

#[test]
fn every_declared_metadata_is_consistent() {
    for meta in Command::metas() {
        meta.check().unwrap();
    }
}

#[test]
fn every_command_is_exactly_one_mcp_tool_with_a_description_and_both_schemas() {
    // Declared by the macro (independent of schemars) versus derived from the schema.
    let declared = Command::metas();
    let tools = catalog();
    assert_eq!(
        tools.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
        declared.iter().map(|m| m.name).collect::<Vec<_>>(),
        "a command without a tool, or a tool without a command"
    );
    for (tool, meta) in tools.iter().zip(&declared) {
        assert!(
            tool.description.len() > 20,
            "{} lacks a description",
            tool.name
        );
        assert_eq!(
            (tool.kind, tool.undo, tool.destructive),
            (meta.kind, meta.undo, meta.destructive),
            "{}: schema metadata differ from meta()",
            tool.name
        );
        assert_eq!(tool.input_schema["type"], "object", "{}", tool.name);
        assert!(
            tool.input_schema["properties"].get("type").is_none(),
            "{}",
            tool.name
        );
        assert_eq!(
            tool.output_schema["type"], "object",
            "{}: no output variant of the same name",
            tool.name
        );
        assert!(
            tool.output_schema["properties"].get("type").is_none(),
            "{}",
            tool.name
        );
    }
}

#[test]
fn examples_round_trip_through_json_and_answer_with_their_name() {
    for (command, from) in examples() {
        let json = serde_json::to_string(&command).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["type"], command.name(), "{json}");
        let back: Command = serde_json::from_str(&json).unwrap();
        assert_eq!(back, command, "{json}");
        let outcome = command.apply(&mut from.clone(), &mut context()).unwrap();
        assert_eq!(outcome.output.name(), command.name());
        let out = serde_json::to_value(&outcome.output).unwrap();
        assert_eq!(out["type"], command.name(), "{out}");
        let out_back: Output = serde_json::from_value(out).unwrap();
        assert_eq!(out_back, outcome.output);
    }
}

fn parse(json: &str) -> Result<Command, String> {
    serde_json::from_str::<Command>(json).map_err(|e| e.to_string())
}

#[test]
fn malformed_commands_are_refused_with_a_useful_message() {
    let cases = [
        (r#"{"name":"x"}"#, "string `type` field"),
        (r#"{"type":12}"#, "string `type` field"),
        (r#"{"type":"rename"}"#, "not a `domain.verb`"),
        (
            r#"{"type":"song.rename","name":"x"}"#,
            "unknown command domain `song`",
        ),
        (
            r#"{"type":"project.renmae","name":"x"}"#,
            "unknown variant `project.renmae`",
        ),
        (
            r#"{"type":"track.explode"}"#,
            "unknown variant `track.explode`",
        ),
        (
            r#"{"type":"project.rename","name":"x","extra":1}"#,
            "unknown field `extra`",
        ),
        (r#"{"type":"project.rename"}"#, "missing field `name`"),
        (
            r#"{"type":"project.rename","track":null}"#,
            "unknown field `track`",
        ),
        (
            r#"{"type":"view.select_track","track":"X"}"#,
            "26 characters",
        ),
        (
            r#"{"type":"view.select_track","track":"01ARZ3NDEKTSV4RRFFQ69G5FAI"}"#,
            "'I'",
        ),
        (
            r#"{"type":"view.select_track","track":"8ARZ3NDEKTSV4RRFFQ69G5FAV0"}"#,
            "starts with 0 to 7",
        ),
        (r#"{"type":"view.select_track"}"#, "missing field `track`"),
        ("[]", "string `type` field"),
        ("null", "string `type` field"),
    ];
    for (json, expected) in cases {
        let err = parse(json).expect_err(json);
        assert!(err.contains(expected), "{json}: {err}");
    }
    let unknown = parse(r#"{"type":"song.rename"}"#).unwrap_err();
    assert!(unknown.contains("project, track, device"), "{unknown}");
}

#[test]
fn export_audio_refusals_leave_the_state_untouched() {
    let export = |region, sample_rate, from_bar, to_bar| {
        Command::Project(project::Command::ExportAudio {
            region,
            bit_depth: None,
            sample_rate,
            from_bar,
            to_bar,
        })
    };
    let invalid = |command: &Command, state: &State, field: &str| {
        let err = check_refusal(command, state, &context()).unwrap();
        assert!(
            matches!(err, CommandError::Invalid { field: ref f, .. } if f == field),
            "{err:?}"
        );
    };
    let arranged = arrangement_examples::fx().state;
    // Nothing in the arrangement: nothing to render, for the song and for bars.
    invalid(&export(None, None, None, None), &state(), "region");
    invalid(
        &export(Some(ExportRegion::Bars), None, Some(1), Some(2)),
        &state(),
        "from_bar",
    );
    // The loop region of the loop's export has to exist.
    invalid(
        &export(Some(ExportRegion::Loop), None, None, None),
        &arranged,
        "region",
    );
    // A sample rate a file cannot have.
    for rate in [0, 8_000, 48_001, 192_000] {
        invalid(
            &export(None, Some(rate), None, None),
            &arranged,
            "sample_rate",
        );
    }
    // The three rates a file can have.
    for rate in [44_100, 48_000, 96_000] {
        check(&export(None, Some(rate), None, None), &arranged, &context()).unwrap();
    }
    // Bars: both or neither, in order, counted from 1, and with the bars region only.
    let bars = |from, to| export(Some(ExportRegion::Bars), None, from, to);
    invalid(&bars(None, None), &arranged, "from_bar");
    invalid(&bars(Some(2), None), &arranged, "from_bar");
    invalid(&bars(Some(0), Some(2)), &arranged, "from_bar");
    invalid(&bars(Some(3), Some(2)), &arranged, "to_bar");
    invalid(&export(None, None, Some(1), Some(2)), &arranged, "region");
    invalid(
        &export(Some(ExportRegion::Loop), None, Some(1), Some(2)),
        &arranged,
        "region",
    );
    // The fixture's arrangement ends in bar 3 (11 520 ticks): from bar 4 there is nothing.
    invalid(&bars(Some(4), Some(5)), &arranged, "from_bar");
    check(&bars(Some(3), Some(9)), &arranged, &context()).unwrap();
    check(&bars(Some(1), Some(1)), &arranged, &context()).unwrap();
    // A bar so far that the export would last for days is a limit, not a failure to encode.
    let err = check_refusal(&bars(Some(1), Some(u32::MAX)), &arranged, &context()).unwrap();
    assert_eq!(err.code(), "limit");
}

#[test]
fn an_export_of_more_than_thirty_minutes_is_a_limit_not_a_bug_of_the_caller() {
    // 1 000 bars at 120 BPM in 4/4 are 2 000 s: more than the 1 800 s the export takes.
    let arranged = arrangement_examples::fx().state;
    let command = Command::Project(project::Command::ExportAudio {
        region: Some(ExportRegion::Bars),
        bit_depth: None,
        sample_rate: None,
        from_bar: Some(1),
        to_bar: Some(1_000),
    });
    let mut long = arranged.clone();
    // The arrangement reaches that far (a placement is only a clip and its extent).
    let track = long.project.tracks.values_mut().next().unwrap();
    let placement = track.placements.values_mut().next().unwrap().clone();
    let far = ::project::Placement {
        start: ::project::Tick(1_000 * 3840 - 960),
        ..placement
    };
    track.placements.insert(
        ::project::PlacementId::generate(&mut ::project::IdGenerator::new(1, 7)),
        far,
    );
    let err = check_refusal(&command, &long, &context()).unwrap();
    assert!(
        matches!(err, CommandError::Limit { ref entity, max } if entity == "export seconds" && max == 1_800),
        "{err:?}"
    );
    assert_eq!(err.code(), "limit");
    // 900 bars are 1 800 s: the longest one that goes.
    let ok = Command::Project(project::Command::ExportAudio {
        region: Some(ExportRegion::Bars),
        bit_depth: None,
        sample_rate: None,
        from_bar: Some(1),
        to_bar: Some(900),
    });
    check(&ok, &long, &context()).unwrap();
}

#[test]
fn a_refused_command_leaves_the_state_untouched() {
    let empty = Command::Project(project::Command::Rename { name: "   ".into() });
    let err = check_refusal(&empty, &state(), &context()).unwrap();
    assert_eq!(err.code(), "invalid");
    assert!(matches!(err, CommandError::Invalid { ref field, .. } if field == "name"));
    let long = Command::Project(project::Command::Rename {
        name: "x".repeat(project::MAX_NAME_CHARS + 1),
    });
    check_refusal(&long, &state(), &context()).unwrap();
    let max = Command::Project(project::Command::Rename {
        name: "é".repeat(project::MAX_NAME_CHARS),
    });
    check(&max, &state(), &context()).unwrap();
}

#[test]
fn rename_undo_is_exact_from_the_examples_state_and_at_the_boundaries() {
    let rename = |name: &str| Command::Project(project::Command::Rename { name: name.into() });
    check(&rename("Demo"), &state(), &context()).unwrap();
    let mut boundary = state();
    boundary.project.name = "x".repeat(project::MAX_NAME_CHARS);
    check(&rename("Demo"), &boundary, &context()).unwrap();
    check(&rename("  padded  "), &state(), &context()).unwrap();
}

#[test]
fn rename_to_the_current_name_changes_nothing_and_undoes_nothing() {
    let mut s = state();
    let outcome = Command::Project(project::Command::Rename {
        name: " Untitled ".into(),
    })
    .apply(&mut s, &mut context())
    .unwrap();
    assert_eq!(s, state());
    assert_eq!(outcome.undo, commands::Undo::None);
    assert!(outcome.events.is_empty());
    assert_eq!(
        outcome.output,
        Output::Project(project::Output::Rename {
            previous: "Untitled".into()
        })
    );
}

#[test]
fn rename_trims_reports_the_previous_name_and_its_inverse_restores_it() {
    let mut s = state();
    let mut c = context();
    let outcome = Command::Project(project::Command::Rename {
        name: "  Demo  ".into(),
    })
    .apply(&mut s, &mut c)
    .unwrap();
    assert_eq!(s.project.name, "Demo");
    assert_eq!(
        outcome.output,
        Output::Project(project::Output::Rename {
            previous: "Untitled".into()
        })
    );
    assert_eq!(
        outcome.events,
        [ProjectEvent::ProjectRenamed {
            name: "Demo".into()
        }]
    );
    assert_eq!(
        outcome.meta_of_undo(),
        Some(Meta {
            name: "project.rename",
            kind: Kind::Edit,
            undo: UndoKind::Inverse,
            destructive: false
        })
    );
}

/// Reads the inverse's metadata, if any.
trait UndoMeta {
    fn meta_of_undo(&self) -> Option<Meta>;
}

impl UndoMeta for commands::Outcome {
    fn meta_of_undo(&self) -> Option<Meta> {
        match &self.undo {
            commands::Undo::Inverse(c) => Some(c.meta()),
            commands::Undo::None => None,
        }
    }
}

#[test]
fn select_track_is_session_only_and_reports_the_previous_selection() {
    let mut s = fixture().state;
    let project = s.project.clone();
    let track = fixture().track;
    let mut c = context();
    let select = Command::View(view::Command::SelectTrack {
        track: Some(track.clone()),
    });
    let first = select.apply(&mut s, &mut c).unwrap();
    assert_eq!(s.session.selected_track, Some(track.clone()));
    assert_eq!(s.project, project);
    assert_eq!(
        first.output,
        Output::View(view::Output::SelectTrack { previous: None })
    );
    let second = Command::View(view::Command::SelectTrack { track: None })
        .apply(&mut s, &mut c)
        .unwrap();
    assert_eq!(
        second.output,
        Output::View(view::Output::SelectTrack {
            previous: Some(track.clone())
        })
    );
    assert_eq!(s.session.selected_track, None);
    assert_eq!(second.events, [ProjectEvent::TrackSelected { track: None }]);
}

#[test]
fn authors_serialize_as_user_or_agent_name() {
    let user: Author = serde_json::from_str(r#""user""#).unwrap();
    assert_eq!(user, Author::User);
    let agent: Author = serde_json::from_str(r#""agent:claude""#).unwrap();
    assert_eq!(
        agent,
        Author::Agent {
            name: "claude".into()
        }
    );
    assert_eq!(serde_json::to_string(&agent).unwrap(), r#""agent:claude""#);
    assert!(serde_json::from_str::<Author>(r#""agent:""#).is_err());
    assert!(serde_json::from_str::<Author>(r#""robot""#).is_err());
}

#[test]
fn errors_carry_a_stable_code() {
    let err = CommandError::NotFound {
        entity: "track".into(),
        id: TRACK.into(),
    };
    let json = serde_json::to_value(&err).unwrap();
    assert_eq!(json["code"], "not_found");
    assert_eq!(err.code(), "not_found");
    assert_eq!(err.to_string(), format!("track {TRACK} not found"));
}

#[test]
fn the_examples_project_id_never_meets_a_generated_id() {
    // The examples' state and context are seeded apart: the ids the commands generate
    // never collide with the project's own id (one id space per project).
    let project = state().project.id.0;
    let mut ctx = context();
    for _ in 0..1000 {
        assert_ne!(ctx.ids.next_id(), project);
    }
}

#[test]
fn live_and_device_refusals_leave_the_state_untouched() {
    let f = fixture();
    let s = &f.state;
    let unknown = track();
    let refused = |c: Command, code: &str| {
        let err = check_refusal(&c, s, &context()).unwrap();
        assert_eq!(err.code(), code, "{}: {err}", c.name());
    };
    refused(
        Command::Live(live::Command::NoteOn {
            track: unknown.clone(),
            note: 60,
            velocity: 1,
        }),
        "not_found",
    );
    refused(
        Command::Live(live::Command::NoteOff {
            track: unknown,
            note: 60,
        }),
        "not_found",
    );
    for (note, velocity) in [(128, 1), (60, 128)] {
        refused(
            Command::Live(live::Command::NoteOn {
                track: f.track.clone(),
                note,
                velocity,
            }),
            "invalid",
        );
    }
    refused(
        Command::Live(live::Command::NoteOff {
            track: f.track.clone(),
            note: 128,
        }),
        "invalid",
    );
    let set = |device: ::project::DeviceId, param: &str, value: f64| {
        Command::Device(device::Command::SetParam {
            device,
            param: param.into(),
            value,
        })
    };
    let missing = ::project::DeviceId::parse(TRACK).unwrap();
    refused(set(missing, "attack", 0.1), "not_found");
    refused(set(f.device.clone(), "cutoff", 0.1), "not_found");
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        refused(set(f.device.clone(), "attack", value), "invalid");
    }
}

#[test]
fn set_param_to_the_current_value_changes_nothing() {
    let f = fixture();
    let mut s = f.state.clone();
    let current = device::param_defaults()
        .into_iter()
        .find(|(n, _)| *n == "gain")
        .map(|(_, v)| f64::from(v))
        .unwrap();
    let out = Command::Device(device::Command::SetParam {
        device: f.device.clone(),
        param: "gain".into(),
        value: current,
    })
    .apply(&mut s, &mut context())
    .unwrap();
    assert_eq!(s, f.state);
    assert_eq!(out.undo, commands::Undo::None);
    assert!(out.events.is_empty());
}

#[test]
fn every_synth_parameter_has_a_unique_engine_id_and_its_serde_name() {
    let ids = device::param_ids();
    assert_eq!(ids.len(), device::SynthParam::ALL.len());
    for (param, (name, id)) in device::SynthParam::ALL.iter().zip(&ids) {
        assert_eq!(serde_json::to_value(param).unwrap(), *name);
        assert_eq!(device::SynthParam::from_name(name), Some(*param));
        assert_eq!(engine::Param::from_id(*id), Some(param.to_engine()));
    }
    let mut unique: Vec<_> = ids.iter().map(|(_, id)| id).collect();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), ids.len());
    assert_eq!(device::SynthParam::from_name("cutoff"), None);
}

#[test]
fn midi_select_input_is_session_only() {
    let f = fixture();
    let mut s = f.state.clone();
    let c = Command::Midi(midi::Command::SelectInput {
        id: Some("in-1".into()),
    });
    let out = c.apply(&mut s, &mut context()).unwrap();
    assert_eq!(s.session.midi_input.as_deref(), Some("in-1"));
    assert_eq!(s.project, f.state.project);
    assert_eq!(
        out.events,
        [ProjectEvent::MidiInputSelected {
            id: Some("in-1".into())
        }]
    );
    // The same input again: nothing to tell.
    assert!(c.apply(&mut s, &mut context()).unwrap().events.is_empty());
    // `id` may be left out, as in v1.
    assert_eq!(
        parse(r#"{"type":"midi.select_input"}"#).unwrap(),
        Command::Midi(midi::Command::SelectInput { id: None })
    );
}

#[test]
fn stored_project_commands_name_something_the_page_can_look_up() {
    let named = |value: &str| {
        [
            Command::Project(project::Command::OpenRecent { id: value.into() }),
            Command::Project(project::Command::OpenBrowserProject { dir: value.into() }),
            Command::Project(project::Command::DeleteBrowserProject { dir: value.into() }),
        ]
    };
    for command in named("") {
        let err = check_refusal(&command, &state(), &context()).unwrap();
        assert_eq!(err.code(), "invalid", "{}", command.name());
    }
    for command in named(&"x".repeat(129)) {
        let err = check_refusal(&command, &state(), &context()).unwrap();
        assert!(
            matches!(err, CommandError::Invalid { ref message, .. } if message.contains("128")),
            "{}: {err}",
            command.name()
        );
    }
    // The page's own identifiers (a UUID) and the longest name accepted pass.
    for value in [
        "3f2b8c1e-0a52-4c3d-9b7e-1d2f4a6b8c0e".to_owned(),
        "é".repeat(128),
    ] {
        for command in named(&value) {
            check(&command, &state(), &context()).unwrap();
        }
    }
    // The field named in the refusal is the one the command has.
    let id = Command::Project(project::Command::OpenRecent { id: String::new() });
    let err = check_refusal(&id, &state(), &context()).unwrap();
    assert!(matches!(err, CommandError::Invalid { ref field, .. } if field == "id"));
    let dir = Command::Project(project::Command::OpenBrowserProject { dir: String::new() });
    let err = check_refusal(&dir, &state(), &context()).unwrap();
    assert!(matches!(err, CommandError::Invalid { ref field, .. } if field == "dir"));
}

#[test]
fn open_url_takes_https_or_this_computer_and_refuses_everything_else() {
    let open = |url: &str| {
        Command::Project(project::Command::OpenUrl {
            url: url.to_owned(),
        })
    };
    for url in [
        "https://cdn.jsdelivr.net/gh/sprkl/faulk-journey@main/tracks/a/project.faulkpack",
        "HTTPS://Example.org/song.faulkpack?v=2#top",
        "https://example.org:8443/a.faulkpack",
        "https://xn--bcher-kva.example/a.faulkpack",
        "http://localhost:5497/song.faulkpack",
        "http://LOCALHOST/song.faulkpack",
        "http://127.0.0.1:9/x",
        "http://[::1]:1/x",
        "http://localhost",
    ] {
        check(&open(url), &state(), &context()).unwrap_or_else(|e| panic!("{url}: {e}"));
    }
    let long = format!("https://example.org/{}", "a".repeat(2048));
    for url in [
        "",
        long.as_str(),
        "example.org/song.faulkpack",
        "//example.org/song.faulkpack",
        "ftp://example.org/song.faulkpack",
        "file:///C:/song.faulkpack",
        "javascript:alert(1)//://",
        "data:application/zip;base64,UEsDBA==",
        "blob:https://faulk.app/1234",
        "http://example.org/song.faulkpack",
        "http://localhost.example.org/song.faulkpack",
        "http://127.0.0.1.nip.io/song.faulkpack",
        "http://localhost@example.org/song.faulkpack",
        "https://user:secret@example.org/song.faulkpack",
        "http://example.org\\@localhost/song.faulkpack",
        "http://example.org#@localhost",
        "https:///song.faulkpack",
        "https://:443/song.faulkpack",
        "https://example.org:80a/song.faulkpack",
        "http://[::1/song.faulkpack",
        "http://[::1]x/song.faulkpack",
        "https://example.org/a song.faulkpack",
        " https://example.org/song.faulkpack",
        "https://example.org/song.faulkpack\n",
    ] {
        let err = check_refusal(&open(url), &state(), &context())
            .unwrap_or_else(|e| panic!("{url:?}: {e}"));
        assert!(
            matches!(err, CommandError::Invalid { ref field, .. } if field == "url"),
            "{url:?}: {err}"
        );
    }
    // The longest link accepted passes.
    let longest = format!("https://example.org/{}", "a".repeat(2048 - 20));
    assert_eq!(longest.chars().count(), 2048);
    check(&open(&longest), &state(), &context()).unwrap();
}

#[test]
fn a_command_that_can_lose_work_is_never_declared_destructive_and_says_the_page_asks() {
    // `Meta::check` allows `destructive` on an edit only, and these are live: the flag cannot
    // carry the guarantee, so the description does (the page asks, whoever sends the command).
    for name in [
        "project.delete_browser_project",
        "project.keep_mine",
        "project.load_folder_version",
    ] {
        let tool = catalog().into_iter().find(|t| t.name == name).unwrap();
        assert!(!tool.destructive, "{name}");
        let description = tool
            .description
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        assert!(
            description.contains("always asks the user to confirm"),
            "{name}: {description}"
        );
    }
}
