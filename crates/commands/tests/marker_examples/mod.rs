//! The section markers (#473): a fixture with two markers, and one example of each command.

use ::project::{Marker, MarkerId, Tick};
use commands::testing::fixture;
use commands::{Command, State, arrangement};

pub struct Fx {
    pub state: State,
    pub verse: MarkerId,
    pub chorus: MarkerId,
}

pub fn fx() -> Fx {
    let mut state = fixture().state;
    let verse = MarkerId::parse("01HF0000000000000000000M01").unwrap();
    let chorus = MarkerId::parse("01HF0000000000000000000M02").unwrap();
    state.project.markers.insert(
        verse.clone(),
        Marker {
            tick: Tick(0),
            name: "Verse".into(),
        },
    );
    state.project.markers.insert(
        chorus.clone(),
        Marker {
            tick: Tick(3840 * 8),
            name: "Chorus".into(),
        },
    );
    state.project.validate().unwrap();
    Fx {
        state,
        verse,
        chorus,
    }
}

pub fn m(c: arrangement::Command) -> Command {
    Command::Arrangement(c)
}

pub fn examples() -> Vec<(Command, State)> {
    let f = fx();
    let on = |c: arrangement::Command| (m(c), f.state.clone());
    let mut gone = f.state.clone();
    gone.project.markers.remove(&f.chorus);
    vec![
        on(arrangement::Command::AddMarker {
            tick: Tick(3840 * 16),
            name: Some("  Bridge ".into()),
        }),
        on(arrangement::Command::AddMarker {
            tick: Tick(3840 * 4),
            name: None,
        }),
        on(arrangement::Command::RenameMarker {
            marker: f.verse.clone(),
            name: "Intro".into(),
        }),
        on(arrangement::Command::MoveMarker {
            marker: f.chorus.clone(),
            tick: Tick(3840 * 12),
        }),
        on(arrangement::Command::ListMarkers {}),
        on(arrangement::Command::DeleteMarker {
            marker: f.verse.clone(),
        }),
        (
            m(arrangement::Command::RestoreMarker {
                marker: f.chorus.clone(),
                tick: Tick(3840 * 8),
                name: "Chorus".into(),
            }),
            gone,
        ),
    ]
}
