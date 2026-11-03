//! A big project (#245): thousands of clips over many tracks, whose snapshot is longer than
//! the engine takes at once (its rt ring and its prepare queue), built by commands. Shared
//! by the paced-send tests of the harness and of the helper.

use commands::clip::ClipTarget;
use commands::{Command, Context, Output, State, clip, scene, testing, track};
use project::{Note, SceneId, Tick, TrackId};

use crate::scenario::Session;

/// One bar in 4/4.
const BAR: i64 = 3840;

/// The note every clip plays (half a bar from its start).
pub const PITCH: u8 = 60;

/// A big project and what a test needs to play it.
pub struct BigProject {
    pub state: State,
    pub ctx: Context,
    /// The tracks in display order.
    pub tracks: Vec<TrackId>,
    /// The scenes in order: every slot of the launch grid holds a clip.
    pub scenes: Vec<SceneId>,
}

impl BigProject {
    /// Clips in the project.
    pub fn clips(&self) -> usize {
        self.state
            .project
            .tracks
            .values()
            .map(|t| t.clips.len())
            .sum()
    }

    /// A session (and its engine) set up from this project: a `Reset` and its whole
    /// snapshot in one push.
    pub fn session(&self) -> Session {
        Session::from_state(self.state.clone(), self.ctx.clone())
    }
}

/// `tracks` MIDI tracks playing the built-in synth, `scenes` scenes with a one-bar clip in
/// every slot of the launch grid, and `bars` one-bar clips one after the other in each
/// track's arrangement: `tracks * (scenes + bars)` clips of one note each. Built by commands:
/// the first track is filled clip by clip, then duplicated (`track.duplicate`).
pub fn big_project(tracks: usize, scenes: usize, bars: usize) -> BigProject {
    let fixture = testing::fixture();
    let mut s = Session::from_state(fixture.state.clone(), testing::context());
    let first = fixture.track.clone();
    let mut scene_ids = Vec::with_capacity(scenes);
    for _ in 0..scenes {
        let Output::Scene(scene::Output::Create { scene }) =
            s.exec(Command::Scene(scene::Command::Create { name: None }))
        else {
            panic!("a scene")
        };
        scene_ids.push(scene);
    }
    let targets = scene_ids
        .iter()
        .map(|scene| ClipTarget::Slot {
            scene: scene.clone(),
        })
        .chain((0..bars).map(|bar| ClipTarget::Arrangement {
            start: Tick(bar as i64 * BAR),
        }));
    for at in targets {
        let Output::Clip(clip::Output::Create { clip, .. }) =
            s.exec(Command::Clip(clip::Command::Create {
                track: first.clone(),
                at,
                length: Some(Tick(BAR)),
                name: None,
            }))
        else {
            panic!("a clip")
        };
        s.run(Command::Clip(clip::Command::NotesAdd {
            clip,
            notes: vec![Note::new(Tick(0), PITCH, Tick(BAR / 2), 100)],
        }));
    }
    let mut track_ids = vec![first.clone()];
    for _ in 1..tracks {
        let Output::Track(track::Output::Duplicate { track }) =
            s.exec(Command::Track(track::Command::Duplicate {
                track: first.clone(),
                name: None,
            }))
        else {
            panic!("a track")
        };
        track_ids.push(track);
    }
    BigProject {
        state: s.state().clone(),
        ctx: s.ctx().clone(),
        tracks: track_ids,
        scenes: scene_ids,
    }
}
