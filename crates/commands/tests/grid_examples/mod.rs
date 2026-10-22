//! One example of every branch of the `scene.*` and `launch.*` commands, with the state it
//! runs from (the clip tests' fixture: three scenes, a clip in the first, a clip with a
//! placement in the second, an audio clip). `tests/contract.rs` adds [`examples`] to the
//! catalog of examples (a command without one fails there); `tests/scene.rs` also checks
//! each against `validate`.

#![allow(dead_code)]

use crate::clip_examples::{ClipFx, after, fx};
use commands::testing::fixture;
use commands::{Command, State, launch, scene};
use project::OrderKey;

pub fn sc(command: scene::Command) -> Command {
    Command::Scene(command)
}

pub fn la(command: launch::Command) -> Command {
    Command::Launch(command)
}

/// The clip tests' fixture.
pub fn grid() -> ClipFx {
    fx()
}

pub fn examples() -> Vec<(Command, State)> {
    let f = fx();
    let on = |c: Command| (c, f.state.clone());
    let empty = fixture().state;
    let first = f.state.project.scenes[&f.scene_a].order.clone();
    let last = f.state.project.scenes[&f.scene_free].order.clone();
    // `scene.restore`: the inverse of a delete, of a scene with a clip and of one whose
    // clip has a placement.
    let restored = |scene| {
        let (state, inverse) = after(f.state.clone(), &sc(scene::Command::Delete { scene }));
        (inverse, state)
    };
    let named = after(
        f.state.clone(),
        &sc(scene::Command::Rename {
            scene: f.scene_a.clone(),
            name: "Verse".into(),
        }),
    )
    .0;
    vec![
        restored(f.scene_a.clone()),
        restored(f.scene_b.clone()),
        restored(f.scene_free.clone()),
        (sc(scene::Command::Create { name: None }), empty.clone()),
        on(sc(scene::Command::Create {
            name: Some("  Chorus  ".into()),
        })),
        // An empty scene, one with a clip, one with a clip and a placement.
        on(sc(scene::Command::Delete {
            scene: f.scene_free.clone(),
        })),
        on(sc(scene::Command::Delete {
            scene: f.scene_a.clone(),
        })),
        on(sc(scene::Command::Delete {
            scene: f.scene_b.clone(),
        })),
        on(sc(scene::Command::Rename {
            scene: f.scene_a.clone(),
            name: "  Verse  ".into(),
        })),
        (
            sc(scene::Command::Rename {
                scene: f.scene_a.clone(),
                name: String::new(),
            }),
            named.clone(),
        ),
        on(sc(scene::Command::Move {
            scene: f.scene_a.clone(),
            index: 2,
        })),
        on(sc(scene::Command::Move {
            scene: f.scene_free.clone(),
            index: 0,
        })),
        on(sc(scene::Command::RestoreOrder {
            scene: f.scene_a.clone(),
            order: OrderKey::between(Some(&first), Some(&last)).unwrap(),
        })),
        on(la(launch::Command::Clip {
            clip: f.clip.clone(),
        })),
        on(la(launch::Command::Clip {
            clip: f.placed.clone(),
        })),
        on(la(launch::Command::Scene {
            scene: f.scene_a.clone(),
        })),
        on(la(launch::Command::Scene {
            scene: f.scene_free.clone(),
        })),
        on(la(launch::Command::StopTrack {
            track: f.track.clone(),
        })),
        on(la(launch::Command::StopAll {})),
    ]
}
