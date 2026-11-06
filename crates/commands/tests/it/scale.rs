//! #695: the project's key and scale. `project.set_scale` sets it (chromatic is none),
//! refuses a root outside 0 to 11, changes nothing on the same value, and undoes; the
//! scale travels through `project.get`.

use ::project::{Scale, ScaleKind, ScaleMode};
use commands::project::{Command as P, Output as PO};
use commands::testing::{check, check_refusal, context, fixture};
use commands::{Command, CommandError, Output, ProjectEvent, State, Undo, history, run};

fn set(root: u8, mode: ScaleKind) -> Command {
    Command::Project(P::SetScale { root, mode })
}

fn get(state: &mut State) -> (u8, ScaleKind, Vec<u8>) {
    match run(&Command::Project(P::Get {}), state, &mut context())
        .unwrap()
        .output
    {
        Output::Project(PO::Get {
            scale_root,
            scale_mode,
            scale_pitch_classes,
            ..
        }) => (scale_root, scale_mode, scale_pitch_classes),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_new_project_has_no_scale_and_reads_as_chromatic() {
    let mut s = fixture().state;
    assert_eq!(s.project.scale, None);
    assert_eq!(get(&mut s), (0, ScaleKind::Chromatic, (0..12).collect()));
}

#[test]
fn set_scale_sets_it_answers_the_previous_one_and_undoes() {
    let f = fixture();
    let command = set(5, ScaleKind::NaturalMinor);
    check(&command, &f.state, &context()).unwrap();
    let mut s = f.state.clone();
    let out = run(&command, &mut s, &mut context()).unwrap();
    assert_eq!(
        s.project.scale,
        Some(Scale {
            root: 5,
            mode: ScaleMode::NaturalMinor
        })
    );
    assert_eq!(
        out.output,
        Output::Project(PO::SetScale {
            previous_root: 0,
            previous_mode: ScaleKind::Chromatic
        })
    );
    assert_eq!(
        out.events.first(),
        Some(&ProjectEvent::ScaleChanged {
            root: 5,
            mode: ScaleKind::NaturalMinor
        })
    );
    assert_eq!(
        out.undo,
        Undo::Inverse(Box::new(set(0, ScaleKind::Chromatic)))
    );
    // F minor: F G A♭ B♭ C D♭ E♭.
    assert_eq!(get(&mut s).2, vec![0, 1, 3, 5, 7, 8, 10]);
    s.project.validate().unwrap();

    // From one scale to another, the inverse names the first one.
    let out = run(&set(2, ScaleKind::Dorian), &mut s, &mut context()).unwrap();
    assert_eq!(
        out.undo,
        Undo::Inverse(Box::new(set(5, ScaleKind::NaturalMinor)))
    );

    // Undo twice: back to no scale; redo twice: D dorian again.
    let undo = Command::History(history::Command::Undo {});
    let redo = Command::History(history::Command::Redo {});
    run(&undo, &mut s, &mut context()).unwrap();
    assert_eq!(get(&mut s).1, ScaleKind::NaturalMinor);
    run(&undo, &mut s, &mut context()).unwrap();
    assert_eq!(s.project.scale, None);
    run(&redo, &mut s, &mut context()).unwrap();
    run(&redo, &mut s, &mut context()).unwrap();
    let (root, mode, _) = get(&mut s);
    assert_eq!((root, mode), (2, ScaleKind::Dorian));
}

#[test]
fn the_same_scale_changes_nothing() {
    let mut s = fixture().state;
    // No scale, then chromatic: unchanged whatever the root (chromatic keeps none).
    for root in [0, 7, 11] {
        let out = run(&set(root, ScaleKind::Chromatic), &mut s, &mut context()).unwrap();
        assert_eq!(out.undo, Undo::None);
        assert!(out.events.is_empty());
    }
    run(&set(9, ScaleKind::Blues), &mut s, &mut context()).unwrap();
    let out = run(&set(9, ScaleKind::Blues), &mut s, &mut context()).unwrap();
    assert_eq!(out.undo, Undo::None);
    assert!(out.events.is_empty());
    // Another root with the same mode is a change.
    let out = run(&set(10, ScaleKind::Blues), &mut s, &mut context()).unwrap();
    assert!(matches!(out.undo, Undo::Inverse(_)));
    // Back to chromatic clears it and keeps no root.
    run(&set(4, ScaleKind::Chromatic), &mut s, &mut context()).unwrap();
    assert_eq!(s.project.scale, None);
    assert_eq!(get(&mut s).0, 0);
}

#[test]
fn a_root_outside_the_twelve_pitch_classes_is_refused() {
    let f = fixture();
    for root in [12, 13, 255] {
        for mode in [ScaleKind::Major, ScaleKind::Chromatic] {
            let err = check_refusal(&set(root, mode), &f.state, &context()).unwrap();
            assert!(
                matches!(err, CommandError::Invalid { ref field, .. } if field == "root"),
                "{err:?}"
            );
        }
    }
}

#[test]
fn every_scale_kind_names_its_mode_and_back() {
    // Chromatic is none; every other kind is one mode, and every mode has its kind.
    assert_eq!(ScaleKind::Chromatic.mode(), None);
    for mode in ScaleMode::ALL {
        let kind = ScaleKind::from(Some(mode));
        assert_eq!(kind.mode(), Some(mode));
        assert_eq!(
            serde_json::to_value(kind).unwrap(),
            serde_json::to_value(mode).unwrap()
        );
        assert_eq!(serde_json::to_value(mode).unwrap(), mode.name());
    }
}

#[test]
fn the_command_reads_as_the_issue_writes_it() {
    let json = r#"{"type":"project.set_scale","root":5,"mode":"natural_minor"}"#;
    let c: Command = serde_json::from_str(json).unwrap();
    assert_eq!(c, set(5, ScaleKind::NaturalMinor));
    let none = r#"{"type":"project.set_scale","root":0,"mode":"chromatic"}"#;
    assert_eq!(
        serde_json::from_str::<Command>(none).unwrap(),
        set(0, ScaleKind::Chromatic)
    );
    for bad in [
        r#"{"type":"project.set_scale","root":5,"mode":"ionian_sharp"}"#,
        r#"{"type":"project.set_scale","mode":"major"}"#,
        r#"{"type":"project.set_scale","root":-1,"mode":"major"}"#,
        r#"{"type":"project.set_scale","root":5,"mode":"major","extra":1}"#,
    ] {
        assert!(serde_json::from_str::<Command>(bad).is_err(), "{bad}");
    }
}

/// #690: the engine hears the scale as a set of pitch classes: on a change, on its undo (none
/// is 0), and in the snapshot of a project that has one (none sends nothing: a `Reset` clears
/// it).
#[test]
fn the_engine_hears_the_scale_as_its_pitch_classes() {
    use commands::ControlMsg;
    use commands::engine_msgs::{Synced, snapshot};
    let scales = |msgs: &[ControlMsg]| -> Vec<u16> {
        msgs.iter()
            .filter_map(|m| match m {
                ControlMsg::SetScale { pitch_classes } => Some(*pitch_classes),
                _ => None,
            })
            .collect()
    };
    let f = fixture();
    let (msgs, _) = snapshot(&f.state.project, &f.state.session);
    assert!(scales(&msgs).is_empty(), "no scale, nothing sent");
    let mut synced = Synced::new(f.state.clone(), context(), 1);
    synced.setup().unwrap();
    // A minor: C D E F G A B.
    let (out, _) = synced.run(&set(9, ScaleKind::NaturalMinor)).unwrap();
    assert_eq!(scales(&out.control), [0b1010_1011_0101]);
    let (msgs, _) = snapshot(&synced.state.project, &synced.state.session);
    assert_eq!(scales(&msgs), [0b1010_1011_0101]);
    // F minor: F G A flat B flat C D flat E flat.
    let (out, _) = synced.run(&set(5, ScaleKind::NaturalMinor)).unwrap();
    assert_eq!(scales(&out.control), [0b0101_1010_1011]);
    let (out, _) = synced.run(&set(0, ScaleKind::Chromatic)).unwrap();
    assert_eq!(scales(&out.control), [0]);
    let (out, _) = synced
        .run(&Command::History(history::Command::Undo {}))
        .unwrap();
    assert_eq!(scales(&out.control), [0b0101_1010_1011]);
}
