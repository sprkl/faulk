//! AU-1: `clip.import_audio`, `clip.delete` and `clip.restore_audio` on audio clips, and
//! `clip.choose_audio`: what the clip holds, the undo that keeps the file, every refusal
//! leaving the state as it was, and the staged files forgotten with their project.

use crate::clip_examples::{ClipFx, after, c, fx, with_audio_placement};
use ::project::{AudioMedia, ClipContent, Frame, FrameLoop, Hash, TrackKind};
use commands::clip::{self, AudioClipSnapshot, AudioPlacementSnapshot};
use commands::testing::{bare_context, check_refusal, context, imported_media};
use commands::{Command, CommandError, Output, ProjectEvent, Undo, project, run};

fn import(f: &ClipFx, media: Hash, name: Option<&str>) -> Command {
    c(clip::Command::ImportAudio {
        track: f.audio_track.clone(),
        media,
        name: name.map(str::to_owned),
        at: None,
    })
}

fn code(error: CommandError) -> String {
    serde_json::to_value(&error).unwrap()["code"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[test]
fn an_imported_file_becomes_a_whole_clip_in_the_pool_and_undo_keeps_the_file() {
    let f = fx();
    let mut state = f.state.clone();
    let mut ctx = context();
    let media = imported_media();
    let outcome = run(&import(&f, media.hash, None), &mut state, &mut ctx).unwrap();
    let Output::Clip(clip::Output::ImportAudio { clip, .. }) = &outcome.output else {
        panic!("{:?}", outcome.output)
    };
    let track = state.project.tracks[&f.audio_track].clone();
    let made = &track.clips[clip];
    // The file's name without its extension; the whole file, 0 dB, no loop; in the pool.
    assert_eq!(made.name, "Kick 01");
    assert_eq!((made.slot.clone(), made.color), (None, None));
    assert!(!track.placements.values().any(|p| p.clip == *clip));
    assert_eq!(
        made.content,
        ClipContent::Audio {
            media: media.clone(),
            start: Frame::new(0),
            frames: media.frames,
            gain_db: 0.0,
            looping: None,
        }
    );
    assert!(
        outcome.events.iter().any(|e| matches!(e, ProjectEvent::ClipCreated { clip: c, placements, .. } if c == clip && placements.is_empty()))
    );
    state.project.validate().unwrap();
    // Undo: the clip goes; nothing of the file is in the project to remove (the blob stays in
    // the folder, the project only ever referenced it).
    let Undo::Inverse(undo) = outcome.undo else {
        panic!("no inverse")
    };
    assert_eq!(*undo, c(clip::Command::Delete { clip: clip.clone() }));
    let redo = undo.apply(&mut state, &mut ctx).unwrap();
    assert_eq!(state.project, f.state.project);
    // Redo brings back the same clip, with its identifier, from the snapshot alone (no
    // staging needed).
    let Undo::Inverse(redo) = redo.undo else {
        panic!("no redo")
    };
    let mut fresh = bare_context();
    redo.apply(&mut state, &mut fresh).unwrap();
    assert_eq!(state.project.tracks[&f.audio_track].clips[clip], *made);
}

#[test]
fn a_file_the_project_already_uses_needs_no_new_import() {
    let f = fx();
    let mut state = f.state.clone();
    // The fixture's audio clip plays `Hash([7; 32])`; nothing is staged.
    let out = run(
        &import(&f, Hash([7; 32]), Some("  Again ")),
        &mut state,
        &mut bare_context(),
    )
    .unwrap();
    let Output::Clip(clip::Output::ImportAudio { clip, .. }) = out.output else {
        panic!()
    };
    let made = &state.project.tracks[&f.audio_track].clips[&clip];
    assert_eq!(made.name, "Again");
    let ClipContent::Audio {
        media,
        start,
        frames,
        ..
    } = &made.content
    else {
        panic!()
    };
    assert_eq!(media.name, "loop.wav");
    assert_eq!(
        (start.get(), frames.get()),
        (0, 88_200),
        "the whole file, not the other clip's range"
    );
}

#[test]
fn the_staged_metadata_wins_over_a_clips_record_of_the_same_file() {
    let f = fx();
    let mut ctx = bare_context();
    let staged = AudioMedia {
        hash: Hash([7; 32]),
        name: "loop (decoded).wav".into(),
        ..imported_media()
    };
    ctx.media.insert(staged.hash, staged.clone());
    let mut state = f.state.clone();
    let out = run(&import(&f, staged.hash, None), &mut state, &mut ctx).unwrap();
    let Output::Clip(clip::Output::ImportAudio { clip, .. }) = out.output else {
        panic!()
    };
    let ClipContent::Audio { media, .. } =
        &state.project.tracks[&f.audio_track].clips[&clip].content
    else {
        panic!()
    };
    assert_eq!(*media, staged);
}

#[test]
fn every_refusal_of_an_import_leaves_the_state_as_it_was() {
    let f = fx();
    let refused = |command: &Command| code(check_refusal(command, &f.state, &context()).unwrap());
    // A MIDI track, an unknown track.
    let midi = c(clip::Command::ImportAudio {
        track: f.track.clone(),
        media: imported_media().hash,
        name: None,
        at: None,
    });
    assert_eq!(refused(&midi), "invalid");
    let mut ids = ::project::IdGenerator::new(9, 9);
    let ghost = c(clip::Command::ImportAudio {
        track: ::project::TrackId::generate(&mut ids),
        media: imported_media().hash,
        name: None,
        at: None,
    });
    assert_eq!(refused(&ghost), "not_found");
    // A file nobody imported.
    assert_eq!(refused(&import(&f, Hash([1; 32]), None)), "invalid");
    // A name too long, or untrimmed past its spaces (it is trimmed: fine).
    assert_eq!(
        refused(&import(&f, imported_media().hash, Some(&"x".repeat(201)))),
        "invalid"
    );
    // A staged file whose metadata the project would refuse.
    let mut ctx = context();
    let bad = AudioMedia {
        hash: Hash([2; 32]),
        channels: 0,
        ..imported_media()
    };
    ctx.media.insert(bad.hash, bad);
    assert_eq!(
        code(check_refusal(&import(&f, Hash([2; 32]), None), &f.state, &ctx).unwrap()),
        "invalid"
    );
}

#[test]
fn a_default_name_comes_from_the_files_name() {
    let f = fx();
    for (file, expected) in [
        ("Kick 01.wav", "Kick 01"),
        ("take.final.flac", "take.final"),
        (".hidden", ".hidden"),
        ("no extension", "no extension"),
        ("Song name. Part 2", "Song name. Part 2"),
        ("  spaced  .mp3", "spaced"),
        ("", ""),
    ] {
        let mut ctx = bare_context();
        let media = AudioMedia {
            name: file.into(),
            ..imported_media()
        };
        ctx.media.insert(media.hash, media);
        let mut state = f.state.clone();
        let out = run(
            &import(&f, imported_media().hash, None),
            &mut state,
            &mut ctx,
        );
        // A file's name the project would refuse (untrimmed) is refused with it.
        if file.trim() != file {
            assert!(out.is_err(), "{file:?}");
            continue;
        }
        let Output::Clip(clip::Output::ImportAudio { clip, .. }) = out.unwrap().output else {
            panic!()
        };
        assert_eq!(
            state.project.tracks[&f.audio_track].clips[&clip].name, expected,
            "{file:?}"
        );
    }
    // A file named at the longest a name may be: the clip takes its stem.
    let mut ctx = bare_context();
    let media = AudioMedia {
        name: format!("{}.wav", "y".repeat(196)),
        ..imported_media()
    };
    ctx.media.insert(media.hash, media);
    let mut state = f.state.clone();
    let out = run(
        &import(&f, imported_media().hash, None),
        &mut state,
        &mut ctx,
    )
    .unwrap();
    let Output::Clip(clip::Output::ImportAudio { clip, .. }) = out.output else {
        panic!()
    };
    assert_eq!(
        state.project.tracks[&f.audio_track].clips[&clip]
            .name
            .chars()
            .count(),
        196
    );
}

#[test]
fn deleting_an_audio_clip_and_its_placements_restores_them_with_their_ids() {
    let f = fx();
    let (placed, placement) = with_audio_placement(&f);
    let (deleted, restore) = after(
        placed.clone(),
        &c(clip::Command::Delete {
            clip: f.audio_clip.clone(),
        }),
    );
    assert!(
        !deleted.project.tracks[&f.audio_track]
            .clips
            .contains_key(&f.audio_clip)
    );
    assert!(deleted.project.tracks[&f.audio_track].placements.is_empty());
    let Command::Clip(clip::Command::RestoreAudio {
        placements,
        snapshot,
        ..
    }) = &restore
    else {
        panic!("{restore:?}")
    };
    assert_eq!(placements.len(), 1);
    assert_eq!(placements[0].id, placement);
    assert_eq!(
        (placements[0].length.get(), placements[0].offset.get()),
        (22_050, 11_025)
    );
    assert_eq!(snapshot.name, "Loop");
    let mut state = deleted.clone();
    run(&restore, &mut state, &mut bare_context()).unwrap();
    assert_eq!(state.project, placed.project);
}

fn snapshot(f: &ClipFx) -> AudioClipSnapshot {
    let Command::Clip(clip::Command::RestoreAudio { snapshot, .. }) = after(
        f.state.clone(),
        &c(clip::Command::Delete {
            clip: f.audio_clip.clone(),
        }),
    )
    .1
    else {
        panic!()
    };
    snapshot
}

#[test]
fn every_refusal_of_a_restore_leaves_the_state_as_it_was() {
    let f = fx();
    let (deleted, _) = after(
        f.state.clone(),
        &c(clip::Command::Delete {
            clip: f.audio_clip.clone(),
        }),
    );
    let good = snapshot(&f);
    let restore = |track: &::project::TrackId,
                   snap: AudioClipSnapshot,
                   placements: Vec<AudioPlacementSnapshot>| {
        c(clip::Command::RestoreAudio {
            track: track.clone(),
            clip: f.audio_clip.clone(),
            snapshot: snap,
            placements,
        })
    };
    let refused = |command: Command, from: &commands::State| {
        code(check_refusal(&command, from, &context()).unwrap())
    };
    // The fixture's state still holds the clip: its identifier is in use.
    assert_eq!(
        refused(restore(&f.audio_track, good.clone(), vec![]), &f.state),
        "invalid"
    );
    // A MIDI track.
    assert_eq!(
        refused(restore(&f.track, good.clone(), vec![]), &deleted),
        "invalid"
    );
    // The snapshot breaks the project's rules.
    let broken: Vec<AudioClipSnapshot> = vec![
        AudioClipSnapshot {
            frames: Frame::new(0),
            ..good.clone()
        },
        AudioClipSnapshot {
            start: Frame::new(88_000),
            ..good.clone()
        },
        AudioClipSnapshot {
            gain_db: f64::NAN,
            ..good.clone()
        },
        AudioClipSnapshot {
            looping: Some(FrameLoop {
                start: Frame::new(10),
                end: Frame::new(10),
            }),
            ..good.clone()
        },
        AudioClipSnapshot {
            looping: Some(FrameLoop {
                start: Frame::new(0),
                end: Frame::new(44_101),
            }),
            ..good.clone()
        },
        AudioClipSnapshot {
            media: AudioMedia {
                channels: 65,
                ..good.media.clone()
            },
            ..good.clone()
        },
        AudioClipSnapshot {
            media: AudioMedia {
                name: " x".into(),
                ..good.media.clone()
            },
            ..good.clone()
        },
        AudioClipSnapshot {
            name: "x".repeat(201),
            ..good.clone()
        },
        AudioClipSnapshot {
            slot: Some(f.scene_free.clone()),
            ..good.clone()
        }, // free in the MIDI track, a scene of the project: fine
    ];
    for (i, snap) in broken.into_iter().enumerate() {
        let command = restore(&f.audio_track, snap, vec![]);
        if i == 8 {
            // A free slot of an existing scene is a valid place to bring it back.
            let mut s = deleted.clone();
            run(&command, &mut s, &mut context()).unwrap();
            continue;
        }
        assert_eq!(refused(command, &deleted), "invalid", "case {i}");
    }
    // A placement off the timeline, of no length, or past the clip.
    let mut ids = ::project::IdGenerator::new(11, 11);
    let p = |start: i64, length: u64, offset: u64, ids: &mut ::project::IdGenerator| {
        AudioPlacementSnapshot {
            id: ::project::PlacementId::generate(ids),
            start: ::project::Tick(start),
            length: Frame::new(length),
            offset: Frame::new(offset),
        }
    };
    for placement in [
        p(-1, 10, 0, &mut ids),
        p(0, 0, 0, &mut ids),
        p(0, 10, 44_100, &mut ids),
    ] {
        assert_eq!(
            refused(
                restore(&f.audio_track, good.clone(), vec![placement]),
                &deleted
            ),
            "invalid"
        );
    }
    // A placement's identifier twice, or the clip's own.
    let twice = p(0, 10, 0, &mut ids);
    assert_eq!(
        refused(
            restore(&f.audio_track, good.clone(), vec![twice.clone(), twice]),
            &deleted
        ),
        "invalid"
    );
}

#[test]
fn choosing_audio_files_is_for_an_audio_track_and_changes_nothing() {
    let f = fx();
    let mut state = f.state.clone();
    let out = run(
        &c(clip::Command::ChooseAudio {
            track: f.audio_track.clone(),
        }),
        &mut state,
        &mut bare_context(),
    )
    .unwrap();
    assert!(matches!(out.undo, Undo::None) && out.events.is_empty());
    assert_eq!(state, f.state);
    let midi = c(clip::Command::ChooseAudio {
        track: f.track.clone(),
    });
    assert_eq!(
        code(check_refusal(&midi, &f.state, &context()).unwrap()),
        "invalid"
    );
    assert_eq!(
        f.state.project.tracks[&f.audio_track].kind,
        TrackKind::Audio
    );
}

#[test]
fn a_new_or_opened_project_forgets_the_files_imported_into_the_old_one() {
    for command in [
        Command::Project(project::Command::New { name: None }),
        Command::Project(project::Command::Open {
            folder: commands::testing::OPENED_FOLDER.into(),
        }),
    ] {
        let mut state = fx().state;
        let mut ctx = context();
        assert!(!ctx.media.is_empty());
        run(&command, &mut state, &mut ctx).unwrap();
        assert!(ctx.media.is_empty(), "{}", command.name());
    }
}
