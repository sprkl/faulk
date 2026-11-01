//! A two-track clip project (CL-2) and a two-scene grid (GR-1): built and launched by
//! commands (`scene.*`, `launch.*`, GR-2), with the transport stopped at first: the first
//! launch starts it. Shared by the engine's render tests, the native/WASM parity tests and
//! the listening renders.

use commands::testing;
use commands::{Command, Output, clip, launch, scene, track, transport};
use engine::msg::{ControlMsg, SceneSlot, TrackSlot};
use project::{Note, Tick, TickLoop};

use crate::scenario::{Scenario, Session};

/// When the second track's clip starts: the first bar line after its launch, 2 s at
/// 120 BPM in 4/4.
pub const SECOND_START_SECS: f32 = 2.0;
/// When `transport.stop` is sent.
pub const STOP_SECS: f32 = 6.0;
/// The length of the render.
pub const END_SECS: f32 = 7.0;

fn n(start: i64, pitch: u8, length: i64, velocity: u8) -> Note {
    Note {
        start: Tick(start),
        pitch,
        length: Tick(length),
        velocity,
        cents: 0,
        chance: Note::DEFAULT_CHANCE,
        repeat: Note::DEFAULT_REPEAT,
    }
}

/// The project: the synth track of `commands::testing::fixture` and its duplicate, one
/// scene; a one-bar bass loop on the first track, launched at once; a two-bar clip on the
/// second (an intro bar, then a loop over most of its second bar, a note with fine tuning, a note
/// crossing the loop end), launched during the first bar when `second` is true; the
/// transport stops at [`STOP_SECS`].
pub fn two_tracks(second: bool) -> Scenario {
    let fixture = testing::fixture();
    let mut s = Session::from_state(fixture.state.clone(), testing::context());
    let Output::Scene(scene::Output::Create { scene }) =
        s.exec(Command::Scene(scene::Command::Create {
            name: Some("A".into()),
        }))
    else {
        panic!("a scene")
    };
    let bass = fixture.track.clone();
    let Output::Track(track::Output::Duplicate { track: lead }) =
        s.exec(Command::Track(track::Command::Duplicate {
            track: bass.clone(),
            name: Some("Lead".into()),
        }))
    else {
        panic!("a track")
    };
    let mut clips = Vec::new();
    for (track, length) in [(&bass, 3840), (&lead, 7680)] {
        let Output::Clip(clip::Output::Create { clip, .. }) =
            s.exec(Command::Clip(clip::Command::Create {
                track: track.clone(),
                at: clip::ClipTarget::Slot {
                    scene: scene.clone(),
                },
                length: Some(Tick(length)),
                name: None,
            }))
        else {
            panic!("a clip")
        };
        clips.push(clip);
    }
    let bass_notes = vec![
        n(0, 36, 400, 110),
        n(960, 43, 400, 90),
        n(1920, 36, 400, 110),
        n(2880, 46, 700, 90),
    ];
    let mut tuned = n(5760, 69, 900, 80);
    tuned.cents = 30;
    let lead_notes = vec![
        n(0, 64, 1800, 70),
        n(1920, 67, 1800, 70),
        n(3840, 72, 900, 85),
        n(4800, 71, 900, 85),
        tuned,
        // Crosses the loop end (7200): cut at each wrap.
        n(6720, 76, 900, 75),
    ];
    for (clip, notes, looping) in [
        (&clips[0], bass_notes, (0, 3840)),
        (&clips[1], lead_notes, (3840, 7200)),
    ] {
        s.run(Command::Clip(clip::Command::NotesAdd {
            clip: clip.clone(),
            notes,
        }));
        s.run(Command::Clip(clip::Command::SetLoop {
            clip: clip.clone(),
            looping: Some(TickLoop {
                start: Tick(looping.0),
                end: Tick(looping.1),
            }),
        }));
    }
    // The transport is stopped: the launch starts it, and the bass starts at once.
    s.run(Command::Launch(launch::Command::Clip {
        clip: clips[0].clone(),
    }));
    s.render_secs(1.3);
    if second {
        s.run(Command::Launch(launch::Command::Clip {
            clip: clips[1].clone(),
        }));
    }
    s.render_secs(STOP_SECS - 1.3);
    s.run(Command::Transport(transport::Command::Stop {}));
    s.render_secs(END_SECS - STOP_SECS);
    s.into_scenario()
}

/// When the note is added in [`pad_edit`].
pub const PAD_EDIT_SECS: f32 = 0.5;
/// How long the pad of [`pad_edit`] sounds: 3000 ticks, 1.5625 s at 120 BPM.
pub const PAD_END_SECS: f32 = 1.5625;
/// Where the pad is certainly still sounding if the edit did not cut it: twice the default
/// release of the synth (0.3 s) after the edit, so that the tail of a cut note (the added note
/// included) is long gone and a peak in the window can only come from the pad itself.
pub const PAD_SOUNDING_FROM_SECS: f32 = PAD_EDIT_SECS + 0.6;
/// The length of the render of [`pad_edit`].
pub const PAD_RENDER_SECS: f32 = 2.5;

/// A one-bar loop with a long note (a pad) on the first track, launched at once; at
/// [`PAD_EDIT_SECS`], while the pad sounds, `clip.notes_add` adds a short note that ranks
/// before it in the clip (#434: the pad keeps sounding, it is not cut).
pub fn pad_edit() -> Scenario {
    pad_scenario(3000)
}

/// The control of [`pad_edit`]: the pad is 960 ticks long, so it ends at [`PAD_EDIT_SECS`], as
/// if the edit had cut it. The window [`PAD_SOUNDING_FROM_SECS`]..[`PAD_END_SECS`] must be
/// silent here and sound in [`pad_edit`]: the margin that tells a cut note from a continued one.
pub fn pad_cut() -> Scenario {
    pad_scenario(960)
}

fn pad_scenario(pad_ticks: i64) -> Scenario {
    let fixture = testing::fixture();
    let mut s = Session::from_state(fixture.state.clone(), testing::context());
    let Output::Scene(scene::Output::Create { scene }) =
        s.exec(Command::Scene(scene::Command::Create {
            name: Some("A".into()),
        }))
    else {
        panic!("a scene")
    };
    let Output::Clip(clip::Output::Create { clip, .. }) =
        s.exec(Command::Clip(clip::Command::Create {
            track: fixture.track.clone(),
            at: clip::ClipTarget::Slot { scene },
            length: Some(Tick(3840)),
            name: None,
        }))
    else {
        panic!("a clip")
    };
    s.run(Command::Clip(clip::Command::NotesAdd {
        clip: clip.clone(),
        notes: vec![n(0, 48, pad_ticks, 100)],
    }));
    s.run(Command::Clip(clip::Command::SetLoop {
        clip: clip.clone(),
        looping: Some(TickLoop {
            start: Tick(0),
            end: Tick(3840),
        }),
    }));
    s.run(Command::Launch(launch::Command::Clip {
        clip: clip.clone(),
    }));
    s.render_secs(PAD_EDIT_SECS);
    s.run(Command::Clip(clip::Command::NotesAdd {
        clip,
        notes: vec![n(0, 30, 100, 100)],
    }));
    s.render_secs(PAD_RENDER_SECS - PAD_EDIT_SECS);
    s.into_scenario()
}

/// When scene B takes over from scene A in [`scene_grid`]: the bar line after its launch.
pub const SCENE_SWITCH_SECS: f32 = 2.0;
/// When `StopAll` takes effect in [`scene_grid`]: the bar line after its order.
pub const STOP_ALL_SECS: f32 = 6.0;
/// The length of the render of [`scene_grid`].
pub const GRID_END_SECS: f32 = 8.0;

/// A two-scene, two-track project (GR-1): each scene holds a one-bar loop on both tracks (a
/// bass and a lead). Scene A is launched while the transport is stopped (the launch starts it), scene B at 1.3 s when `switch`
/// is true (both tracks change at the bar line of [`SCENE_SWITCH_SECS`]), and `StopAll` at
/// 4.3 s when `stop_all` is true (everything stops at [`STOP_ALL_SECS`]). The orders are
/// `launch.*` commands.
pub fn scene_grid(switch: bool, stop_all: bool) -> Scenario {
    let fixture = testing::fixture();
    let mut s = Session::from_state(fixture.state.clone(), testing::context());
    let new_scene = |s: &mut Session, name: &str| {
        let Output::Scene(scene::Output::Create { scene }) =
            s.exec(Command::Scene(scene::Command::Create {
                name: Some(name.into()),
            }))
        else {
            panic!("a scene")
        };
        scene
    };
    let (a, b) = (new_scene(&mut s, "A"), new_scene(&mut s, "B"));
    let bass = fixture.track.clone();
    let Output::Track(track::Output::Duplicate { track: lead }) =
        s.exec(Command::Track(track::Command::Duplicate {
            track: bass.clone(),
            name: Some("Lead".into()),
        }))
    else {
        panic!("a track")
    };
    // (scene, track, notes): one bar each, looped over the whole clip.
    let cells = [
        (&a, &bass, vec![n(0, 36, 1800, 110), n(1920, 43, 1800, 100)]),
        (&a, &lead, vec![n(0, 64, 3600, 70)]),
        (&b, &bass, vec![n(0, 41, 1800, 110), n(1920, 48, 1800, 100)]),
        (&b, &lead, vec![n(0, 69, 3600, 70)]),
    ];
    for (scene, track, notes) in cells {
        let Output::Clip(clip::Output::Create { clip, .. }) =
            s.exec(Command::Clip(clip::Command::Create {
                track: track.clone(),
                at: clip::ClipTarget::Slot {
                    scene: scene.clone(),
                },
                length: Some(Tick(3840)),
                name: None,
            }))
        else {
            panic!("a clip")
        };
        s.run(Command::Clip(clip::Command::NotesAdd {
            clip: clip.clone(),
            notes,
        }));
        s.run(Command::Clip(clip::Command::SetLoop {
            clip,
            looping: Some(TickLoop {
                start: Tick(0),
                end: Tick(3840),
            }),
        }));
    }
    // The transport is stopped: launching scene A starts it.
    s.run(Command::Launch(launch::Command::Scene { scene: a }));
    s.render_secs(1.3);
    if switch {
        s.run(Command::Launch(launch::Command::Scene { scene: b }));
    }
    s.render_secs(3.0);
    if stop_all {
        s.run(Command::Launch(launch::Command::StopAll {}));
    }
    s.render_secs(GRID_END_SECS - 4.3);
    s.into_scenario()
}

/// How a launch reaches an engine whose transport is stopped ([`launch_from_stop`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoppedLaunch {
    /// `launch.clip` alone: the launch and the play go to the engine together.
    Command,
    /// The launch, a block of audio, then the play: what the command's order gives when
    /// the audio thread takes the two messages in different blocks.
    LaunchThenPlay,
    /// The play, a block of audio, then the launch: the order the command does not use.
    PlayThenLaunch,
}

/// When the render of [`launch_from_stop`] ends.
pub const STOPPED_LAUNCH_END_SECS: f32 = 2.5;

/// One synth track, one scene, a one-bar looping clip with a note on the first beat, a
/// stopped transport with its cue at 0 (a bar line): the clip is launched as `how` says.
/// Launched with `launch.clip`, it sounds from the first sample; played before the launch
/// reaches the engine, the playhead has left the cue and the clip waits for the next bar.
pub fn launch_from_stop(how: StoppedLaunch) -> Scenario {
    let fixture = testing::fixture();
    let mut s = Session::from_state(fixture.state.clone(), testing::context());
    let track = fixture.track.clone();
    let Output::Scene(scene::Output::Create { scene }) =
        s.exec(Command::Scene(scene::Command::Create { name: None }))
    else {
        panic!("a scene")
    };
    let Output::Clip(clip::Output::Create { clip, .. }) =
        s.exec(Command::Clip(clip::Command::Create {
            track: track.clone(),
            at: clip::ClipTarget::Slot {
                scene: scene.clone(),
            },
            length: Some(Tick(3840)),
            name: None,
        }))
    else {
        panic!("a clip")
    };
    s.run(Command::Clip(clip::Command::NotesAdd {
        clip: clip.clone(),
        notes: vec![n(0, 57, 1800, 110)],
    }));
    s.run(Command::Clip(clip::Command::SetLoop {
        clip: clip.clone(),
        looping: Some(TickLoop {
            start: Tick(0),
            end: Tick(3840),
        }),
    }));
    let launch = ControlMsg::LaunchClip {
        track: TrackSlot(s.ctx().engine.tracks.get(&track).expect("track slot")),
        scene: SceneSlot(s.ctx().engine.scenes.get(&scene).expect("scene slot") as u16),
    };
    let play = Command::Transport(transport::Command::Play {});
    match how {
        StoppedLaunch::Command => {
            s.run(Command::Launch(launch::Command::Clip { clip }));
        }
        StoppedLaunch::LaunchThenPlay => {
            s.messages(&[launch]).render_blocks(1);
            s.run(play);
        }
        StoppedLaunch::PlayThenLaunch => {
            s.run(play).render_blocks(1);
            s.messages(&[launch]);
        }
    }
    s.render_secs(STOPPED_LAUNCH_END_SECS);
    s.into_scenario()
}
