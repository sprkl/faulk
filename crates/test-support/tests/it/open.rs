//! SV-4: a project saved to a folder and opened again leaves the engine in the state, and
//! renders the sound, of the project it was saved from; nothing of the project it replaces
//! keeps sounding; the solo comes back from the session file and is in no versioned file.

use commands::contract::Folder;
use commands::{Command, Output, clip, launch, scene, track};
use project::file::{MemFs, SaveOptions, SaveState, save, save_session};
use project::{ClipId, Project, SceneId, SessionState, TrackId};
use serde_json::{Value, json};
use test_support::{EngineView, QUANTUM, Render, SAMPLE_RATE, Session, parity, render_native};

fn cmd(v: Value) -> Command {
    serde_json::from_value(v.clone()).unwrap_or_else(|e| panic!("{v}: {e}"))
}

/// What `project::file` writes for a project and its session.
fn folder_fs(project: &Project, session: &SessionState) -> MemFs {
    let mut fs = MemFs::new();
    save(
        &mut fs,
        project,
        &mut SaveState::new(),
        None,
        &SaveOptions::default(),
    )
    .expect("the project saves");
    save_session(&mut fs, session).expect("the session saves");
    fs
}

struct Song {
    session: Session,
    scene: SceneId,
    a: TrackId,
    b: TrackId,
}

/// Two MIDI tracks playing different notes from clips of one scene, `a` a sine on 60 and
/// `b` a saw on 72, built by commands. Nothing plays yet.
fn song() -> Song {
    let mut s = Session::single_synth();
    let a = s.state().project.tracks.keys().next().unwrap().clone();
    let b = match s.exec(cmd(json!({"type": "track.create", "kind": "midi"}))) {
        Output::Track(track::Output::Create { track, .. }) => track,
        other => panic!("{other:?}"),
    };
    let device = |s: &Session, t: &TrackId| {
        s.state().project.tracks[t]
            .devices
            .keys()
            .next()
            .unwrap()
            .clone()
    };
    for (t, waveform) in [(&a, 0.0), (&b, 1.0)] {
        let d = device(&s, t);
        s.run(cmd(
            json!({"type": "device.set_param", "device": d, "param": "waveform", "value": waveform}),
        ));
    }
    let scene = match s.exec(cmd(json!({"type": "scene.create", "name": "A"}))) {
        Output::Scene(scene::Output::Create { scene }) => scene,
        other => panic!("{other:?}"),
    };
    for (t, pitch) in [(&a, 60), (&b, 72)] {
        let clip: ClipId = match s.exec(cmd(json!({
            "type": "clip.create", "track": t, "at": {"at": "slot", "scene": scene}
        }))) {
            Output::Clip(clip::Output::Create { clip, .. }) => clip,
            other => panic!("{other:?}"),
        };
        s.run(cmd(
            json!({"type": "clip.notes.add", "clip": clip, "notes": [
                {"start": 0, "pitch": pitch, "length": 1920, "velocity": 100}
            ]}),
        ));
    }
    Song {
        session: s,
        scene,
        a,
        b,
    }
}

fn play(s: &mut Session, scene: &SceneId, secs: f32) {
    s.run(Command::Launch(launch::Command::Scene {
        scene: scene.clone(),
    }));
    s.render_secs(secs);
}

fn blocks_of(secs: f32) -> usize {
    (secs * SAMPLE_RATE / QUANTUM as f32).ceil() as usize * QUANTUM as usize
}

/// Frames the old project's tracks take to fade out after the `Reset` (5 ms, rounded up
/// to whole blocks): what is heard before them is the old project's last breath, not the
/// opened one.
const FADE_OUT_FRAMES: usize = 2 * QUANTUM as usize;

/// The part of a render from `frame` on.
fn from_frame(render: &Render, frame: usize) -> Render {
    Render::new(
        render.sample_rate,
        render.left[frame..].to_vec(),
        render.right[frame..].to_vec(),
    )
}

/// A session that plays something else, holds a note, and opens `fs` as `name`.
fn open_over_another_project(fs: MemFs, name: &str) -> (Session, usize) {
    let mut s = Session::single_synth();
    s.set_param("release", 0.4)
        .render_secs(0.1)
        .note_on(57, 100)
        .render_secs(0.2);
    let before = blocks_of(0.1) + blocks_of(0.2);
    s.stage(Folder::new(name, fs));
    s.open(name);
    (s, before)
}

#[test]
fn a_project_saved_and_opened_again_renders_as_the_original() {
    let mut original = song();
    play(&mut original.session, &original.scene, 1.0);
    let rendered = render_native(original.session.scenario());
    rendered.assert_clean();
    rendered.assert_audible(0.2..1.0, 0.01);

    let state = original.session.state().clone();
    let fs = folder_fs(&state.project, &state.session);
    let (mut opened, before) = open_over_another_project(fs, "song");
    assert_eq!(opened.state().project, state.project);
    assert_eq!(opened.state().session, state.session);
    play(&mut opened, &original.scene, 1.0);

    // The engine holds what the original's holds, whichever way it got there.
    let view_of = |s: &Session| EngineView::of_stream(s.scenario(), &s.ctx().engine);
    assert_eq!(view_of(&opened), view_of(&original.session));
    assert_eq!(
        view_of(&opened),
        EngineView::of_project(&state.project, &state.session)
    );

    // And plays the same sound, sample for sample, from the Reset on.
    let after = render_native(opened.scenario());
    let tail = from_frame(&after, before + FADE_OUT_FRAMES);
    let (diff, at) = tail.max_difference(&from_frame(&rendered, FADE_OUT_FRAMES));
    assert_eq!(diff, 0.0, "differs at frame {at}");
    // The WebAssembly engine agrees with the native one on the whole sequence.
    parity(opened.scenario());
}

#[test]
fn nothing_of_the_old_project_keeps_sounding() {
    let silent = song();
    let state = silent.session.state().clone();
    let fs = folder_fs(&state.project, &state.session);
    let (mut s, before) = open_over_another_project(fs, "quiet");
    // The opened project plays nothing: no launch.
    s.render_secs(0.5);
    let render = render_native(s.scenario());
    render.assert_clean();
    // The note was held and long to release when the project was replaced.
    let started = before as f32 / SAMPLE_RATE;
    render.assert_audible((started - 0.1)..started, 0.01);
    // The old tracks fade out over 5 ms, then there is nothing: exact zeros, not a tail.
    render.assert_silent((started + 0.02)..render.seconds());
    parity(s.scenario());
}

#[test]
fn the_solo_comes_back_with_the_session_and_is_in_no_versioned_file() {
    let mut song = song();
    song.session.run(cmd(
        json!({"type": "mixer.set_solo", "track": song.b, "solo": true}),
    ));
    let state = song.session.state().clone();
    assert!(state.session.solo.contains(&song.b) && !state.session.solo.contains(&song.a));

    let fs = folder_fs(&state.project, &state.session);
    // The session file has it; no other file does, whatever the word.
    let session = String::from_utf8(fs.files()[".faulk/session.json"].clone()).unwrap();
    assert!(session.contains(&song.b.to_string()), "{session}");
    for (path, bytes) in fs.files() {
        if path.starts_with(".faulk/") {
            continue;
        }
        let text = String::from_utf8_lossy(bytes);
        assert!(!text.contains("solo"), "{path} mentions the solo");
    }
    // The versioned files are the very ones of the project saved without any session.
    let without = folder_fs(&state.project, &SessionState::default());
    for (path, bytes) in without.files() {
        if !path.starts_with(".faulk/") {
            assert_eq!(fs.files().get(path), Some(bytes), "{path}");
        }
    }

    let (mut reopened, _) = open_over_another_project(fs, "soloed");
    assert_eq!(reopened.state().session.solo, state.session.solo);
    let engine = EngineView::of_stream(reopened.scenario(), &reopened.ctx().engine);
    let solo: Vec<bool> = engine.tracks.values().map(|t| t.solo).collect();
    assert_eq!(solo.iter().filter(|s| **s).count(), 1, "{solo:?}");

    // Soloed, it plays what the original plays soloed, and not what it plays unsoloed.
    play(&mut song.session, &song.scene, 1.0);
    let soloed = render_native(song.session.scenario());
    play(&mut reopened, &song.scene, 1.0);
    let (_, before) = (0, blocks_of(0.1) + blocks_of(0.2));
    let reopened_render = from_frame(
        &render_native(reopened.scenario()),
        before + FADE_OUT_FRAMES,
    );
    let (diff, at) = reopened_render.max_difference(&from_frame(&soloed, FADE_OUT_FRAMES));
    assert_eq!(diff, 0.0, "differs at frame {at}");

    let mut everyone = self::song();
    play(&mut everyone.session, &everyone.scene, 1.0);
    let unsoloed = render_native(everyone.session.scenario());
    assert!(
        unsoloed.max_difference(&soloed).0 > 0.01,
        "the solo changes what is heard, so the test means something"
    );
}
