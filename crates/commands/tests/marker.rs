//! `arrangement.marker.*` (#473): the section markers. Every command's refusals leave the state intact,
//! a default name counts up, and inserting or deleting time moves the markers with the time
//! (undo puts them back).

mod marker_examples;

use ::project::{MarkerId, Tick};
use commands::testing::{check, check_refusal, context};
use commands::{Command, CommandError, Output, arrangement, history, run};
use marker_examples::{examples, fx, m};

#[test]
fn every_example_meets_the_contract() {
    for (command, state) in examples() {
        check(&command, &state, &context()).unwrap();
    }
}

fn code(err: CommandError) -> &'static str {
    err.code()
}

#[test]
fn refusals_leave_everything_as_it_was() {
    let f = fx();
    let ghost = MarkerId::parse("01HF0000000000000000000M99").unwrap();
    let cases: Vec<(Command, &str)> = vec![
        (
            m(arrangement::Command::AddMarker {
                tick: Tick(-1),
                name: None,
            }),
            "invalid",
        ),
        (
            m(arrangement::Command::AddMarker {
                tick: Tick((1 << 40) + 1),
                name: None,
            }),
            "invalid",
        ),
        (
            m(arrangement::Command::AddMarker {
                tick: Tick(0),
                name: Some("x".repeat(201)),
            }),
            "invalid",
        ),
        (
            m(arrangement::Command::RenameMarker {
                marker: f.verse.clone(),
                name: "   ".into(),
            }),
            "invalid",
        ),
        (
            m(arrangement::Command::RenameMarker {
                marker: ghost.clone(),
                name: "A".into(),
            }),
            "not_found",
        ),
        (
            m(arrangement::Command::MoveMarker {
                marker: ghost.clone(),
                tick: Tick(0),
            }),
            "not_found",
        ),
        (
            m(arrangement::Command::MoveMarker {
                marker: f.verse.clone(),
                tick: Tick(-5),
            }),
            "invalid",
        ),
        (
            m(arrangement::Command::DeleteMarker { marker: ghost }),
            "not_found",
        ),
        // A restore over an identifier in use.
        (
            m(arrangement::Command::RestoreMarker {
                marker: f.verse.clone(),
                tick: Tick(0),
                name: "Verse".into(),
            }),
            "invalid",
        ),
    ];
    for (command, expected) in cases {
        let err = check_refusal(&command, &f.state, &context()).unwrap();
        assert_eq!(code(err), expected, "{command:?}");
    }
}

#[test]
fn the_markers_are_capped() {
    let f = fx();
    let mut s = f.state.clone();
    let mut ctx = context();
    for i in s.project.markers.len()..::project::limits::MAX_MARKERS {
        run(
            &m(arrangement::Command::AddMarker {
                tick: Tick(i as i64),
                name: Some(format!("M{i}")),
            }),
            &mut s,
            &mut ctx,
        )
        .unwrap();
    }
    let err = check_refusal(
        &m(arrangement::Command::AddMarker {
            tick: Tick(0),
            name: None,
        }),
        &s,
        &context(),
    )
    .unwrap();
    assert_eq!(err.code(), "limit");
}

#[test]
fn a_default_name_takes_the_first_free_number() {
    let f = fx();
    let mut s = f.state.clone();
    let mut ctx = context();
    let mut add = |s: &mut commands::State| {
        let out = run(
            &m(arrangement::Command::AddMarker {
                tick: Tick(0),
                name: Some("  ".into()),
            }),
            s,
            &mut ctx,
        )
        .unwrap();
        let Output::Arrangement(arrangement::Output::AddMarker { marker }) = out.output else {
            panic!()
        };
        s.project.markers[&marker].name.clone()
    };
    assert_eq!(add(&mut s), "Section 1");
    assert_eq!(add(&mut s), "Section 2");
    // Two markers on one tick are allowed.
    assert_eq!(
        s.project
            .markers
            .values()
            .filter(|m| m.tick == Tick(0))
            .count(),
        3
    );
    s.project.validate().unwrap();
}

#[test]
fn time_edits_move_the_markers_and_undo_puts_them_back() {
    let f = fx();
    let mut s = f.state.clone();
    let mut ctx = context();
    let before = s.project.markers.clone();
    // Two bars inserted at bar 5: the chorus (bar 9) moves two bars on, the verse stays.
    run(
        &Command::Arrangement(arrangement::Command::InsertTime {
            at: Tick(3840 * 4),
            length: Tick(3840 * 2),
        }),
        &mut s,
        &mut ctx,
    )
    .unwrap();
    assert_eq!(s.project.markers[&f.verse].tick, Tick(0));
    assert_eq!(s.project.markers[&f.chorus].tick, Tick(3840 * 10));
    run(
        &Command::History(history::Command::Undo {}),
        &mut s,
        &mut ctx,
    )
    .unwrap();
    assert_eq!(s.project.markers, before);
    // Bars 7 to 10 deleted: the chorus, inside, goes to the cut's start.
    run(
        &Command::Arrangement(arrangement::Command::DeleteTime {
            from: Tick(3840 * 6),
            to: Tick(3840 * 10),
        }),
        &mut s,
        &mut ctx,
    )
    .unwrap();
    assert_eq!(s.project.markers[&f.chorus].tick, Tick(3840 * 6));
    run(
        &Command::History(history::Command::Undo {}),
        &mut s,
        &mut ctx,
    )
    .unwrap();
    assert_eq!(s.project.markers, before);
    run(
        &Command::History(history::Command::Redo {}),
        &mut s,
        &mut ctx,
    )
    .unwrap();
    assert_eq!(s.project.markers[&f.chorus].tick, Tick(3840 * 6));
    s.project.validate().unwrap();
}

#[test]
fn the_list_answers_in_timeline_order() {
    let f = fx();
    let mut s = f.state.clone();
    let out = run(&m(arrangement::Command::ListMarkers {}), &mut s, &mut context()).unwrap();
    let Output::Arrangement(arrangement::Output::ListMarkers { markers }) = out.output else {
        panic!()
    };
    let names: Vec<_> = markers.iter().map(|m| (m.name.as_str(), m.tick)).collect();
    assert_eq!(names, [("Verse", Tick(0)), ("Chorus", Tick(3840 * 8))]);
    assert_eq!(markers[0].marker, f.verse);
}
