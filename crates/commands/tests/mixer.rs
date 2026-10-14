//! MX-2: the mixer commands. Contract of the values, fader gestures as one undo group,
//! solo as session state (never in the history nor in the project's serialization),
//! mute as versioned project data.

use ::project::TrackId;
use commands::testing::{check, check_refusal, context, fixture};
use commands::{Command, CommandError, Output, ProjectEvent, State, history, mixer, run};

fn mx(c: mixer::Command) -> Command {
    Command::Mixer(c)
}

fn volume(track: &TrackId, gain_db: Option<f64>) -> Command {
    mx(mixer::Command::SetVolume {
        track: track.clone(),
        gain_db,
    })
}

fn pan(track: &TrackId, pan: f64) -> Command {
    mx(mixer::Command::SetPan {
        track: track.clone(),
        pan,
    })
}

fn mute(track: &TrackId, mute: bool) -> Command {
    mx(mixer::Command::SetMute {
        track: track.clone(),
        mute,
    })
}

fn solo(track: &TrackId, solo: bool) -> Command {
    mx(mixer::Command::SetSolo {
        track: track.clone(),
        solo,
    })
}

fn master(gain_db: Option<f64>) -> Command {
    mx(mixer::Command::SetMasterVolume { gain_db })
}

fn undo() -> Command {
    Command::History(history::Command::Undo {})
}

fn refused(command: &Command, state: &State) -> CommandError {
    check_refusal(command, state, &context()).unwrap()
}

fn invalid_field(error: CommandError) -> String {
    match error {
        CommandError::Invalid { field, .. } => field,
        other => panic!("{other:?}"),
    }
}

#[test]
fn every_command_meets_the_contract_in_each_direction() {
    let f = fixture();
    let t = &f.track;
    // Silence to a level and back, the master too, both pan edges, mute and unmute.
    let mut silent = f.state.clone();
    run(&volume(t, None), &mut silent, &mut context()).unwrap();
    let mut muted = f.state.clone();
    run(&mute(t, true), &mut muted, &mut context()).unwrap();
    let mut soloed = f.state.clone();
    soloed.session.solo.insert(t.clone());
    let cases: Vec<(Command, &State)> = vec![
        (volume(t, Some(-6.0)), &f.state),
        (volume(t, Some(6.0)), &f.state),
        (volume(t, Some(-96.0)), &f.state),
        (volume(t, None), &f.state),
        (volume(t, Some(0.0)), &silent),
        (pan(t, -1.0), &f.state),
        (pan(t, 1.0), &f.state),
        (mute(t, true), &f.state),
        (mute(t, false), &muted),
        (master(Some(-3.0)), &f.state),
        (master(None), &f.state),
        (solo(t, true), &f.state),
        (solo(t, false), &soloed),
    ];
    for (command, state) in cases {
        // Solo is `live`: the harness checks that it leaves the project alone.
        check(&command, state, &context()).unwrap_or_else(|e| panic!("{command:?}: {e}"));
    }
}

#[test]
fn gain_is_refused_outside_its_range_never_clamped() {
    let f = fixture();
    for bad in [
        6.01,
        40.0,
        -96.01,
        -1000.0,
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
    ] {
        for command in [volume(&f.track, Some(bad)), master(Some(bad))] {
            assert_eq!(
                invalid_field(refused(&command, &f.state)),
                "gain_db",
                "{bad}"
            );
        }
    }
    // The bounds themselves are allowed.
    for ok in [mixer::MIN_GAIN_DB, mixer::MAX_GAIN_DB] {
        check(&volume(&f.track, Some(ok)), &f.state, &context()).unwrap();
    }
}

#[test]
fn pan_is_refused_outside_minus_one_to_one() {
    let f = fixture();
    for bad in [1.0001, -1.0001, 5.0, f64::NAN, f64::INFINITY] {
        assert_eq!(
            invalid_field(refused(&pan(&f.track, bad), &f.state)),
            "pan",
            "{bad}"
        );
    }
}

#[test]
fn an_unknown_track_is_not_found_for_every_track_command() {
    let f = fixture();
    let ghost = TrackId::parse("01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap();
    for command in [
        volume(&ghost, Some(0.0)),
        pan(&ghost, 0.0),
        mute(&ghost, true),
        solo(&ghost, true),
    ] {
        assert!(matches!(
            refused(&command, &f.state),
            CommandError::NotFound { .. }
        ));
    }
}

#[test]
fn a_value_already_in_place_changes_nothing_and_records_nothing() {
    let f = fixture();
    let mut s = f.state.clone();
    let depth = s.history.undo.len();
    for command in [
        volume(&f.track, Some(0.0)),
        pan(&f.track, 0.0),
        mute(&f.track, false),
        master(Some(0.0)),
    ] {
        let outcome = run(&command, &mut s, &mut context()).unwrap();
        assert!(outcome.events.is_empty());
    }
    assert_eq!(s, f.state);
    assert_eq!(s.history.undo.len(), depth);
}

#[test]
fn a_fader_gesture_in_one_group_is_one_undo() {
    let f = fixture();
    let mut s = f.state.clone();
    let before = s.project.clone();
    run(
        &Command::History(history::Command::BeginGroup {}),
        &mut s,
        &mut context(),
    )
    .unwrap();
    for db in [-1.0, -2.5, -4.0, -6.0] {
        run(&volume(&f.track, Some(db)), &mut s, &mut context()).unwrap();
    }
    run(
        &Command::History(history::Command::EndGroup {}),
        &mut s,
        &mut context(),
    )
    .unwrap();
    assert_eq!(
        s.history.undo.len(),
        2,
        "the fixture's rename, then the gesture"
    );
    assert_eq!(s.project.tracks[&f.track].mixer.gain_db, -6.0);
    run(&undo(), &mut s, &mut context()).unwrap();
    assert_eq!(s.project, before, "one undo undoes the whole gesture");
}

#[test]
fn a_gesture_groups_through_the_context_too() {
    let f = fixture();
    let mut s = f.state.clone();
    let before = s.project.clone();
    let mut ctx = context();
    ctx.group = Some(s.history.new_group());
    for p in [0.1, 0.5, -0.3] {
        run(&pan(&f.track, p), &mut s, &mut ctx).unwrap();
    }
    run(&undo(), &mut s, &mut context()).unwrap();
    assert_eq!(s.project, before);
}

#[test]
fn undo_restores_silence_and_the_master() {
    let f = fixture();
    let mut s = f.state.clone();
    run(&volume(&f.track, None), &mut s, &mut context()).unwrap();
    assert_eq!(s.project.tracks[&f.track].mixer.gain_db, f64::NEG_INFINITY);
    run(&volume(&f.track, Some(-3.0)), &mut s, &mut context()).unwrap();
    run(&undo(), &mut s, &mut context()).unwrap();
    assert_eq!(
        s.project.tracks[&f.track].mixer.gain_db,
        f64::NEG_INFINITY,
        "undo goes back to silence, not to a number"
    );
    run(&master(Some(-9.0)), &mut s, &mut context()).unwrap();
    run(&undo(), &mut s, &mut context()).unwrap();
    assert_eq!(s.project.master.gain_db, 0.0);
    s.project.validate().unwrap();
}

#[test]
fn a_gain_beyond_the_range_is_refused_before_it_is_edited() {
    // A document edited by hand: +20 dB or -120 dB, which `validate` accepts. A command
    // cannot write that gain back, so it could not undo the edit exactly: it is refused
    // and the state stays intact.
    let f = fixture();
    let mut s = f.state.clone();
    s.project.tracks.get_mut(&f.track).unwrap().mixer.gain_db = 20.0;
    s.project.master.gain_db = -120.0;
    for command in [
        volume(&f.track, Some(0.0)),
        master(Some(0.0)),
        volume(&f.track, None),
    ] {
        assert_eq!(invalid_field(refused(&command, &s)), "gain_db");
    }
    // Pan and mute are not concerned.
    check(&mute(&f.track, true), &s, &context()).unwrap();
}

#[test]
fn outputs_and_events_carry_the_mixer_state() {
    let f = fixture();
    let mut s = f.state.clone();
    let outcome = run(&volume(&f.track, Some(-6.0)), &mut s, &mut context()).unwrap();
    assert_eq!(
        outcome.output,
        Output::Mixer(mixer::Output::SetVolume {
            previous: Some(0.0)
        })
    );
    assert_eq!(
        outcome.events[..1],
        [ProjectEvent::MixerChanged {
            track: Some(f.track.clone()),
            gain_db: Some(-6.0),
            pan: 0.0,
            mute: false,
        }]
    );
    let outcome = run(&master(None), &mut s, &mut context()).unwrap();
    assert_eq!(
        outcome.events[..1],
        [ProjectEvent::MixerChanged {
            track: None,
            gain_db: None,
            pan: 0.0,
            mute: false,
        }]
    );
}

#[test]
fn a_missing_nullable_gain_is_refused_at_parsing() {
    let t = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
    let err =
        serde_json::from_str::<Command>(&format!(r#"{{"type":"mixer.set_volume","track":"{t}"}}"#))
            .unwrap_err()
            .to_string();
    assert!(err.contains("gain_db"), "{err}");
    let ok: Command = serde_json::from_str(&format!(
        r#"{{"type":"mixer.set_volume","track":"{t}","gain_db":null}}"#
    ))
    .unwrap();
    assert!(matches!(
        ok,
        Command::Mixer(mixer::Command::SetVolume { gain_db: None, .. })
    ));
}

#[test]
fn the_schema_states_the_ranges_and_the_kinds() {
    let tools = commands::catalog();
    let tool = |n: &str| tools.iter().find(|t| t.name == n).unwrap();
    assert_eq!(tool("mixer.set_solo").kind, commands::Kind::Live);
    assert_eq!(tool("mixer.set_solo").undo, commands::UndoKind::None);
    for n in [
        "mixer.set_volume",
        "mixer.set_pan",
        "mixer.set_mute",
        "mixer.set_master_volume",
    ] {
        assert_eq!(tool(n).kind, commands::Kind::Edit, "{n}");
        assert_eq!(tool(n).undo, commands::UndoKind::Inverse, "{n}");
        assert!(!tool(n).description.is_empty());
    }
    let pan = &tool("mixer.set_pan").input_schema["properties"]["pan"];
    assert_eq!(
        (pan["minimum"].as_f64(), pan["maximum"].as_f64()),
        (Some(-1.0), Some(1.0))
    );
    for n in ["mixer.set_volume", "mixer.set_master_volume"] {
        let gain = &tool(n).input_schema["properties"]["gain_db"];
        assert_eq!(
            (gain["minimum"].as_f64(), gain["maximum"].as_f64()),
            (Some(-96.0), Some(6.0)),
            "{n}"
        );
        assert!(
            gain["type"].as_array().unwrap().iter().any(|t| t == "null"),
            "{n}: silence is null, the schema must allow it"
        );
    }
    assert!(
        tool("mixer.set_volume").input_schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r == "gain_db")
    );
}

#[test]
fn solo_is_session_state_never_in_the_history_nor_the_project() {
    let f = fixture();
    let mut s = f.state.clone();
    let project_json = serde_json::to_string(&s.project).unwrap();
    let history_before = s.history.clone();
    let outcome = run(&solo(&f.track, true), &mut s, &mut context()).unwrap();
    assert_eq!(
        outcome.events[..1],
        [ProjectEvent::SoloChanged {
            track: f.track.clone(),
            solo: true
        }]
    );
    assert!(s.session.solo.contains(&f.track));
    assert_eq!(s.history, history_before, "solo never enters the history");
    assert_eq!(s.project, f.state.project);
    let after = serde_json::to_string(&s.project).unwrap();
    assert_eq!(after, project_json, "the project text does not change");
    assert!(!after.contains("solo"));
    // Undo goes to the edit before the solo and leaves the solo alone.
    run(&undo(), &mut s, &mut context()).unwrap();
    assert!(s.session.solo.contains(&f.track));
    // Setting it twice is silent; clearing it is an event.
    let again = run(&solo(&f.track, true), &mut s, &mut context()).unwrap();
    assert!(again.events.is_empty());
    assert_eq!(
        again.output,
        Output::Mixer(mixer::Output::SetSolo { previous: true })
    );
    let cleared = run(&solo(&f.track, false), &mut s, &mut context()).unwrap();
    assert!(!cleared.events.is_empty());
    assert!(s.session.solo.is_empty());
}

#[test]
fn mute_is_versioned_in_the_project_text() {
    let f = fixture();
    let mut s = f.state.clone();
    let before = serde_json::to_string(&s.project).unwrap();
    run(&mute(&f.track, true), &mut s, &mut context()).unwrap();
    let after = serde_json::to_string(&s.project).unwrap();
    assert_ne!(before, after, "mute changes the project text");
    assert!(after.contains(r#""mute":true"#));
    assert_eq!(s.history.undo.len(), f.state.history.undo.len() + 1);
    s.project.validate().unwrap();
}
