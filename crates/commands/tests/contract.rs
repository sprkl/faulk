//! The command contract v2 (CT-4), checked on every declared command.
//!
//! The rule: every command has a description, an input and an output schema, and its
//! undo is tested. `every_command_has_an_example` refuses a command that no example
//! covers, and `examples` are checked by the harness of `commands::v2::testing`.

use ::project::TrackId;
use commands::v2::testing::{check, check_refusal, context, state};
use commands::v2::{
    Author, Command, CommandError, Kind, Meta, Output, ProjectEvent, UndoKind, catalog, project,
    view,
};

const TRACK: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";

fn track() -> TrackId {
    TrackId::parse(TRACK).unwrap()
}

/// One example of every command (at least). `every_command_has_an_example` keeps it
/// complete; the harness checks each.
fn examples() -> Vec<Command> {
    vec![
        Command::Project(project::Command::Rename {
            name: "  Demo song  ".into(),
        }),
        Command::View(view::Command::SelectTrack {
            track: Some(track()),
        }),
        Command::View(view::Command::SelectTrack { track: None }),
    ]
}

#[test]
fn every_example_meets_the_contract() {
    for command in examples() {
        check(&command, &state(), &context()).unwrap();
    }
}

#[test]
fn every_command_has_an_example() {
    // The declared commands come from the macro (`METAS`), the examples from this file:
    // two independent lists.
    let mut declared: Vec<_> = Command::metas().iter().map(|m| m.name).collect();
    let mut covered: Vec<_> = examples().iter().map(Command::name).collect();
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
    for command in examples() {
        let json = serde_json::to_string(&command).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["type"], command.name(), "{json}");
        let back: Command = serde_json::from_str(&json).unwrap();
        assert_eq!(back, command, "{json}");
        let outcome = command.apply(&mut state(), &mut context()).unwrap();
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
            r#"{"type":"track.create"}"#,
            "unknown variant `track.create`",
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

impl UndoMeta for commands::v2::Outcome {
    fn meta_of_undo(&self) -> Option<Meta> {
        match &self.undo {
            commands::v2::Undo::Inverse(c) => Some(c.meta()),
            commands::v2::Undo::None => None,
        }
    }
}

#[test]
fn select_track_is_session_only_and_reports_the_previous_selection() {
    let mut s = state();
    let mut c = context();
    let select = Command::View(view::Command::SelectTrack {
        track: Some(track()),
    });
    let first = select.apply(&mut s, &mut c).unwrap();
    assert_eq!(s.session.selected_track, Some(track()));
    assert_eq!(s.project, state().project);
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
            previous: Some(track())
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
