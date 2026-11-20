//! `arrangement.key_change.*` (#953): put, move, remove and list the key changes. Refusals
//! leave the state intact, every command undoes and redoes, the list is capped and sorted,
//! and `project.get` counts them.

use crate::key_change_examples::{BAR, examples, fx, k};
use ::project::{Scale, ScaleKind, ScaleMode, Tick};
use commands::testing::{check, check_refusal, context};
use commands::{Command, Output, arrangement, history, run};

#[test]
fn every_example_meets_the_contract() {
    for (command, state) in examples() {
        check(&command, &state, &context()).unwrap();
    }
}

fn set(bar: i64, root: u8, mode: ScaleKind) -> Command {
    k(arrangement::Command::SetKeyChange {
        tick: Tick(BAR * bar),
        root,
        mode,
    })
}

fn ticks(state: &commands::State) -> Vec<i64> {
    state
        .project
        .key_changes
        .iter()
        .map(|c| c.tick.get())
        .collect()
}

#[test]
fn refusals_leave_everything_as_it_was() {
    let s = fx();
    let cases: Vec<(Command, &str)> = vec![
        // The starting key is project.set_scale's.
        (set(0, 0, ScaleKind::Major), "invalid"),
        // Not a bar start.
        (
            k(arrangement::Command::SetKeyChange {
                tick: Tick(BAR + 960),
                root: 0,
                mode: ScaleKind::Major,
            }),
            "invalid",
        ),
        (
            k(arrangement::Command::SetKeyChange {
                tick: Tick(-BAR),
                root: 0,
                mode: ScaleKind::Major,
            }),
            "invalid",
        ),
        (set(2, 12, ScaleKind::Major), "invalid"),
        (
            k(arrangement::Command::MoveKeyChange {
                from_tick: Tick(BAR * 3),
                to_tick: Tick(BAR * 5),
            }),
            "not_found",
        ),
        // Onto an occupied bar.
        (
            k(arrangement::Command::MoveKeyChange {
                from_tick: Tick(BAR * 4),
                to_tick: Tick(BAR * 8),
            }),
            "invalid",
        ),
        (
            k(arrangement::Command::MoveKeyChange {
                from_tick: Tick(BAR * 4),
                to_tick: Tick(0),
            }),
            "invalid",
        ),
        (
            k(arrangement::Command::RemoveKeyChange { tick: Tick(BAR) }),
            "not_found",
        ),
    ];
    for (command, expected) in cases {
        let err = check_refusal(&command, &s, &context()).unwrap();
        assert_eq!(err.code(), expected, "{command:?}");
    }
}

#[test]
fn set_puts_replaces_and_keeps_the_list_sorted_then_undoes_and_redoes() {
    let mut s = fx();
    let mut ctx = context();
    let before = s.project.key_changes.clone();
    run(&set(2, 9, ScaleKind::NaturalMinor), &mut s, &mut ctx).unwrap();
    assert_eq!(ticks(&s), [BAR * 2, BAR * 4, BAR * 8]);
    // Replacing answers the previous key.
    let out = run(&set(2, 0, ScaleKind::Dorian), &mut s, &mut ctx).unwrap();
    assert_eq!(
        out.output,
        Output::Arrangement(arrangement::Output::SetKeyChange {
            replaced: true,
            previous_root: 9,
            previous_mode: ScaleKind::NaturalMinor
        })
    );
    // The same values: nothing.
    let out = run(&set(2, 0, ScaleKind::Dorian), &mut s, &mut ctx).unwrap();
    assert_eq!(out.undo, commands::Undo::None);
    assert!(out.events.is_empty());
    let undo = Command::History(history::Command::Undo {});
    let redo = Command::History(history::Command::Redo {});
    run(&undo, &mut s, &mut ctx).unwrap();
    assert_eq!(
        s.project.key_changes[0].scale.unwrap().mode,
        ScaleMode::NaturalMinor
    );
    run(&undo, &mut s, &mut ctx).unwrap();
    assert_eq!(s.project.key_changes, before);
    run(&redo, &mut s, &mut ctx).unwrap();
    run(&redo, &mut s, &mut ctx).unwrap();
    assert_eq!(
        s.project.key_changes[0].scale.unwrap().mode,
        ScaleMode::Dorian
    );
    s.project.validate().unwrap();
}

#[test]
fn move_and_remove_undo_and_redo() {
    let mut s = fx();
    let mut ctx = context();
    let before = s.project.key_changes.clone();
    run(
        &k(arrangement::Command::MoveKeyChange {
            from_tick: Tick(BAR * 4),
            to_tick: Tick(BAR * 10),
        }),
        &mut s,
        &mut ctx,
    )
    .unwrap();
    assert_eq!(ticks(&s), [BAR * 8, BAR * 10]);
    run(
        &k(arrangement::Command::RemoveKeyChange {
            tick: Tick(BAR * 8),
        }),
        &mut s,
        &mut ctx,
    )
    .unwrap();
    assert_eq!(ticks(&s), [BAR * 10]);
    let undo = Command::History(history::Command::Undo {});
    run(&undo, &mut s, &mut ctx).unwrap();
    run(&undo, &mut s, &mut ctx).unwrap();
    assert_eq!(s.project.key_changes, before);
    let redo = Command::History(history::Command::Redo {});
    run(&redo, &mut s, &mut ctx).unwrap();
    run(&redo, &mut s, &mut ctx).unwrap();
    assert_eq!(ticks(&s), [BAR * 10]);
}

#[test]
fn the_key_changes_are_capped() {
    let mut s = fx();
    let mut ctx = context();
    let max = ::project::limits::MAX_KEY_CHANGES;
    let mut bar = 100;
    while s.project.key_changes.len() < max {
        run(&set(bar, 0, ScaleKind::Major), &mut s, &mut ctx).unwrap();
        bar += 1;
    }
    let err = check_refusal(&set(2, 0, ScaleKind::Major), &s, &context()).unwrap();
    assert_eq!(err.code(), "limit");
    // Replacing one that is there still works at the cap.
    run(&set(4, 1, ScaleKind::Major), &mut s, &mut ctx).unwrap();
    s.project.validate().unwrap();
}

#[test]
fn the_list_answers_in_order_with_bars_and_pitch_classes() {
    let mut s = fx();
    let out = run(
        &k(arrangement::Command::ListKeyChanges {}),
        &mut s,
        &mut context(),
    )
    .unwrap();
    let Output::Arrangement(arrangement::Output::ListKeyChanges { key_changes }) = out.output
    else {
        panic!()
    };
    assert_eq!(key_changes.len(), 2);
    assert_eq!((key_changes[0].bar, key_changes[0].root), (5, 5));
    // F major: F G A B-flat C D E.
    assert_eq!(key_changes[0].pitch_classes, vec![0, 2, 4, 5, 7, 9, 10]);
    assert_eq!(key_changes[1].mode, ScaleKind::Chromatic);
    assert_eq!(key_changes[1].pitch_classes, (0..12).collect::<Vec<u8>>());
}

#[test]
fn the_scale_in_force_follows_the_changes() {
    let s = fx();
    let at = |bar: i64, tick: i64| s.project.scale_at(Tick(BAR * bar + tick));
    let d = Some(Scale {
        root: 2,
        mode: ScaleMode::Major,
    });
    let f = Some(Scale {
        root: 5,
        mode: ScaleMode::Major,
    });
    assert_eq!(at(0, 0), d);
    assert_eq!(at(3, BAR - 1), d);
    assert_eq!(at(4, 0), f);
    assert_eq!(at(7, BAR - 1), f);
    assert_eq!(at(8, 0), None);
    assert_eq!(at(1000, 0), None);
}

#[test]
fn project_get_counts_the_key_changes() {
    let mut s = fx();
    let out = run(
        &Command::Project(commands::project::Command::Get {}),
        &mut s,
        &mut context(),
    )
    .unwrap();
    let Output::Project(commands::project::Output::Get {
        key_change_count, ..
    }) = out.output
    else {
        panic!()
    };
    assert_eq!(key_change_count, 2);
}
