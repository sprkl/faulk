//! AR-1 through the QA-1 harness: a song built with the arrangement commands only. After
//! each command, each undo and each redo the engine's state, as the stream of messages left
//! it, is the one the snapshot of the project gives; the render has the sound the commands
//! describe; and the native and WASM renders agree. At 120 BPM in 4/4, 960 ticks are 0.5 s.

use commands::{Command, Output, arrangement, clip, history};
use project::{ClipId, PlacementId, Tick, TrackId};
use test_support::{EngineView, Session, parity, render_native};

fn arr(command: arrangement::Command) -> Command {
    Command::Arrangement(command)
}

fn undo() -> Command {
    Command::History(history::Command::Undo {})
}

fn redo() -> Command {
    Command::History(history::Command::Redo {})
}

/// The engine as the live stream left it is the engine the snapshot of the project builds.
fn assert_in_sync(s: &Session, what: &str) {
    let state = s.state();
    let expected = EngineView::of_project(&state.project, &state.session);
    let live = EngineView::of_stream(s.scenario(), &s.ctx().engine);
    assert_eq!(live, expected, "{what}");
}

/// Runs a command, then checks the engine, then undoes and redoes it with the same check.
fn step(s: &mut Session, command: Command, what: &str) -> Output {
    let out = s.exec(command);
    assert_in_sync(s, what);
    s.run(undo());
    assert_in_sync(s, &format!("{what}: undone"));
    s.run(redo());
    assert_in_sync(s, &format!("{what}: redone"));
    out
}

struct Song {
    session: Session,
    track: TrackId,
    clip: ClipId,
    first: PlacementId,
}

/// One bar-long clip at the start with a note at its start (0 to 0.25 s), as `Session`
/// of the fixture's synth track.
fn start() -> Song {
    let mut session = Session::single_synth();
    let track = session
        .state()
        .project
        .tracks
        .keys()
        .next()
        .unwrap()
        .clone();
    let Output::Clip(clip::Output::Create { clip, placement }) =
        session.exec(Command::Clip(clip::Command::Create {
            track: track.clone(),
            at: clip::ClipTarget::Arrangement { start: Tick(0) },
            length: Some(Tick(3840)),
            name: None,
        }))
    else {
        panic!("a clip")
    };
    session.run(Command::Clip(clip::Command::NotesAdd {
        clip: clip.clone(),
        notes: vec![project::Note::new(Tick(0), 60, Tick(480), 100)],
    }));
    Song {
        session,
        track,
        clip,
        first: placement.expect("a placement"),
    }
}

/// The song: the clip copied after itself (independent), the copy moved two bars on, the
/// first placement split, the clip placed again (linked) at 6 s; played to 7.5 s.
fn song() -> Song {
    let mut song = start();
    let s = &mut song.session;
    assert_in_sync(s, "the start");
    let Output::Arrangement(arrangement::Output::Duplicate { placements, .. }) = step(
        s,
        arr(arrangement::Command::Duplicate {
            placements: vec![song.first.clone()],
            linked: false,
            by: None,
        }),
        "duplicate",
    ) else {
        panic!()
    };
    step(
        s,
        arr(arrangement::Command::Move {
            placements: placements.clone(),
            by: Tick(3840),
        }),
        "move",
    );
    step(
        s,
        arr(arrangement::Command::Split {
            placement: song.first.clone(),
            at: Tick(1920),
        }),
        "split",
    );
    step(
        s,
        arr(arrangement::Command::Place {
            clip: song.clip.clone(),
            start: Tick(11520),
            length: None,
            offset: None,
        }),
        "place",
    );
    step(
        s,
        arr(arrangement::Command::Resize {
            placement: song.first.clone(),
            edge: arrangement::Edge::End,
            to: Tick(2400),
        }),
        "resize",
    );
    s.run(Command::Transport(commands::transport::Command::Play {}));
    s.render_secs(7.5);
    song
}

#[test]
fn every_arrangement_command_leaves_the_engine_in_the_state_of_the_snapshot() {
    let mut song = song();
    // The carving rule, the group move and the deletion of the clip's last placement
    // (which deletes the clip) are in sync too.
    let s = &mut song.session;
    step(
        s,
        arr(arrangement::Command::Place {
            clip: song.clip.clone(),
            start: Tick(500),
            length: Some(Tick(4000)),
            offset: Some(Tick(100)),
        }),
        "place over three placements",
    );
    let all: Vec<PlacementId> = s.state().project.tracks[&song.track]
        .placements
        .keys()
        .cloned()
        .collect();
    step(
        s,
        arr(arrangement::Command::Move {
            placements: all.clone(),
            by: Tick(480),
        }),
        "move the whole track",
    );
    step(
        s,
        arr(arrangement::Command::Duplicate {
            placements: all.clone(),
            linked: true,
            by: Some(Tick(50_000)),
        }),
        "linked copy of the whole track",
    );
    step(
        s,
        arr(arrangement::Command::Delete { placements: all }),
        "delete the whole track",
    );
}

#[test]
fn the_song_sounds_where_its_placements_are() {
    let song = song();
    let r = render_native(song.session.scenario());
    r.assert_clean();
    // The note at the start of each placement: 0 s, the copy at 4 s, the linked one at 6 s.
    r.assert_audible(0.05..0.2, 0.05);
    r.assert_audible(4.05..4.2, 0.05);
    r.assert_audible(6.05..6.2, 0.05);
    // Nothing between: the first placement was shortened to 1.2 s and the copy moved away
    // from 2 s.
    r.assert_silent(1.0..3.9);
    r.assert_silent(4.9..5.9);
    r.write_wav("arrangement-commands");
}

#[test]
fn deleting_a_placement_silences_it_and_the_undo_brings_it_back() {
    let mut song = song();
    let s = &mut song.session;
    s.run(Command::Transport(commands::transport::Command::Stop {}));
    s.run(Command::Transport(commands::transport::Command::Seek {
        tick: Tick(0),
    }));
    s.run(arr(arrangement::Command::Delete {
        placements: vec![song.first.clone()],
    }));
    assert_in_sync(s, "delete");
    s.run(Command::Transport(commands::transport::Command::Play {}));
    s.render_secs(1.0);
    s.run(Command::Transport(commands::transport::Command::Stop {}));
    s.run(Command::Transport(commands::transport::Command::Seek {
        tick: Tick(0),
    }));
    s.run(undo());
    assert_in_sync(s, "undo of the delete");
    s.run(Command::Transport(commands::transport::Command::Play {}));
    s.render_secs(1.0);
    let r = render_native(s.scenario());
    r.assert_clean();
    // The song's own render comes first (7.5 s); then 1 s without the placement, where its
    // note would sound at once, then 1 s with it back.
    let start = 7.5;
    r.assert_audible(0.05..0.2, 0.05);
    r.assert_silent(start + 0.05..start + 0.2);
    r.assert_audible(start + 1.05..start + 1.2, 0.05);
}

#[test]
fn the_native_and_wasm_renders_of_a_song_built_with_the_commands_agree() {
    // A short song: the interpreted render is slow.
    let mut song = start();
    let s = &mut song.session;
    s.run(arr(arrangement::Command::Duplicate {
        placements: vec![song.first.clone()],
        linked: false,
        by: Some(Tick(1920)),
    }));
    s.run(Command::Transport(commands::transport::Command::Play {}));
    s.render_secs(1.6);
    let r = parity(s.scenario());
    r.assert_clean();
    r.assert_audible(0.05..0.2, 0.05);
    r.assert_audible(1.05..1.2, 0.05);
}

#[test]
fn a_trimmed_placement_plays_the_clip_from_the_offset_the_command_computed() {
    // A note at the start and one a second in (tick 1920). Cutting the first 960 ticks
    // from the left silences the first note and leaves the second where it was.
    let mut song = start();
    let s = &mut song.session;
    s.run(Command::Clip(clip::Command::NotesAdd {
        clip: song.clip.clone(),
        notes: vec![project::Note::new(Tick(1920), 64, Tick(480), 100)],
    }));
    s.run(arr(arrangement::Command::Resize {
        placement: song.first.clone(),
        edge: arrangement::Edge::Start,
        to: Tick(960),
    }));
    assert_in_sync(s, "resize of the start");
    s.run(Command::Transport(commands::transport::Command::Play {}));
    s.render_secs(2.0);
    let r = render_native(s.scenario());
    r.assert_clean();
    r.assert_silent(0.05..0.9);
    r.assert_audible(1.05..1.2, 0.05);
}

#[test]
fn a_stretched_clip_without_a_loop_repeats_its_content_in_both_builds() {
    // #473: the one-bar clip (a note at its start, 2 s a bar at 120 BPM) stretched to two
    // bars plays its note again at 2 s; the grid's rule (once) is not the arrangement's.
    let mut song = start();
    let s = &mut song.session;
    s.run(arr(arrangement::Command::Resize {
        placement: song.first.clone(),
        edge: arrangement::Edge::End,
        to: Tick(7680),
    }));
    assert_in_sync(s, "stretch");
    s.run(Command::Transport(commands::transport::Command::Play {}));
    s.render_secs(2.6);
    let r = parity(s.scenario());
    r.assert_clean();
    r.assert_audible(0.05..0.2, 0.05);
    r.assert_silent(0.9..1.9);
    r.assert_audible(2.05..2.2, 0.05);
}
