//! The key changes (#953): a fixture with two changes, and one example of each command.

#![allow(dead_code)]

use ::project::{KeyChange, Scale, ScaleMode, Tick};
use commands::testing::fixture;
use commands::{Command, State, arrangement};

/// A bar of the fixture (4/4).
pub const BAR: i64 = 3840;

pub fn fx() -> State {
    let mut state = fixture().state;
    state.project.scale = Some(Scale {
        root: 2,
        mode: ScaleMode::Major,
    });
    state.project.key_changes = vec![
        KeyChange {
            tick: Tick(BAR * 4),
            scale: Some(Scale {
                root: 5,
                mode: ScaleMode::Major,
            }),
        },
        KeyChange {
            tick: Tick(BAR * 8),
            scale: None,
        },
    ];
    state.project.validate().unwrap();
    state
}

pub fn k(c: arrangement::Command) -> Command {
    Command::Arrangement(c)
}

pub fn examples() -> Vec<(Command, State)> {
    let s = fx();
    let on = |c: arrangement::Command| (k(c), s.clone());
    vec![
        on(arrangement::Command::SetKeyChange {
            tick: Tick(BAR * 2),
            root: 9,
            mode: ::project::ScaleKind::NaturalMinor,
        }),
        on(arrangement::Command::SetKeyChange {
            tick: Tick(BAR * 4),
            root: 7,
            mode: ::project::ScaleKind::Dorian,
        }),
        on(arrangement::Command::MoveKeyChange {
            from_tick: Tick(BAR * 4),
            to_tick: Tick(BAR * 6),
        }),
        on(arrangement::Command::RemoveKeyChange {
            tick: Tick(BAR * 8),
        }),
        on(arrangement::Command::ListKeyChanges {}),
    ]
}
