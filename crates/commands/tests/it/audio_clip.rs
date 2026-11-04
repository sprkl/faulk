//! AU-5: an audio clip imported into a slot or the arrangement, its range, gain and loop,
//! the placements that follow its length, the tempo that would push a placement past the
//! timeline, a move in the grid, and a clip whose file the folder lacks ("média absent").

use crate::clip_examples::{ClipFx, after, c, fx, with_audio_placement};
use ::project::file::{FileSystem, MemFs, OpenOptions, SaveOptions, SaveState, open, save};
use ::project::{ClipContent, Frame, FrameLoop, Placement, PlacementId, Span, Tick};
use commands::clip::{self, ClipTarget};
use commands::contract::Folder;
use commands::testing::{bare_context, check_refusal, context, imported_media};
use commands::{Command, Output, State, Undo, run, transport};

fn code(command: &Command, state: &State) -> String {
    check_refusal(command, state, &context())
        .unwrap()
        .code()
        .to_owned()
}

pub(crate) fn import_at(f: &ClipFx, at: Option<ClipTarget>) -> Command {
    c(clip::Command::ImportAudio {
        track: f.audio_track.clone(),
        media: imported_media().hash,
        name: None,
        at,
    })
}

pub(crate) fn range(f: &ClipFx, start: u64, frames: u64) -> Command {
    c(clip::Command::AudioSetRange {
        clip: f.audio_clip.clone(),
        start: Frame::new(start),
        frames: Frame::new(frames),
        placements: None,
    })
}

fn audio(state: &State, f: &ClipFx) -> ClipContent {
    state.project.tracks[&f.audio_track].clips[&f.audio_clip]
        .content
        .clone()
}

/// The fixture with a placement of its audio clip that plays it whole (44 100 frames at
/// 44.1 kHz, one second: 1 920 ticks at 120 BPM) from `start`.
pub(crate) fn with_whole(f: &ClipFx, state: State, start: i64, seed: u128) -> (State, PlacementId) {
    let mut state = state;
    let mut ids = ::project::IdGenerator::new(1_690_000_000_000, seed);
    let id = PlacementId::generate(&mut ids);
    state
        .project
        .tracks
        .get_mut(&f.audio_track)
        .unwrap()
        .placements
        .insert(
            id.clone(),
            Placement {
                clip: f.audio_clip.clone(),
                start: Tick(start),
                span: Span::Frames {
                    length: Frame::new(44_100),
                    offset: Frame::new(0),
                },
            },
        );
    state.project.validate().unwrap();
    (state, id)
}

pub(crate) fn span_of(state: &State, f: &ClipFx, id: &PlacementId) -> Span {
    state.project.tracks[&f.audio_track].placements[id].span
}

#[test]
fn an_import_lands_in_a_slot_or_whole_in_the_arrangement_and_undo_removes_both() {
    let f = fx();
    // In a slot.
    let mut state = f.state.clone();
    let out = run(
        &import_at(
            &f,
            Some(ClipTarget::Slot {
                scene: f.scene_free.clone(),
            }),
        ),
        &mut state,
        &mut context(),
    )
    .unwrap();
    let Output::Clip(clip::Output::ImportAudio { clip, placement }) = &out.output else {
        panic!("{:?}", out.output)
    };
    assert_eq!(placement, &None);
    let track = &state.project.tracks[&f.audio_track];
    assert_eq!(track.clips[clip].slot, Some(f.scene_free.clone()));
    state.project.validate().unwrap();

    // In the arrangement: one placement of the whole file (96 000 frames at 48 kHz).
    let mut state = f.state.clone();
    let mut ctx = context();
    let out = run(
        &import_at(&f, Some(ClipTarget::Arrangement { start: Tick(960) })),
        &mut state,
        &mut ctx,
    )
    .unwrap();
    let Output::Clip(clip::Output::ImportAudio { clip, placement }) = out.output.clone() else {
        panic!()
    };
    let placement = placement.expect("a placement");
    let p = &state.project.tracks[&f.audio_track].placements[&placement];
    assert_eq!(
        (p.clip.clone(), p.start, p.span),
        (
            clip.clone(),
            Tick(960),
            Span::Frames {
                length: Frame::new(96_000),
                offset: Frame::new(0)
            }
        )
    );
    state.project.validate().unwrap();
    // Undo removes the clip and its placement; the media stays known, so the same file
    // makes a clip again without a new import (its blob was never the project's to remove).
    let Undo::Inverse(undo) = out.undo else {
        panic!()
    };
    run(&undo, &mut state, &mut ctx).unwrap();
    assert_eq!(state.project, f.state.project);
    run(&import_at(&f, None), &mut state, &mut ctx).unwrap();
}

#[test]
fn an_import_into_the_arrangement_is_refused_on_an_overlap_or_past_the_timeline() {
    let f = fx();
    // The fixture's placement covers [3840, 3840 + 0.5 s = 4800) at 120 BPM.
    let (placed, _) = with_audio_placement(&f);
    let at = |start: i64| import_at(&f, Some(ClipTarget::Arrangement { start: Tick(start) }));
    // Two seconds of file from 2000 end at 5840: overlap.
    assert_eq!(code(&at(2000), &placed), "invalid");
    assert_eq!(code(&at(4000), &placed), "invalid");
    // Right after it, and well before it (ends at 3840 exactly): fine.
    run(&at(4800), &mut placed.clone(), &mut context()).unwrap();
    run(&at(0), &mut placed.clone(), &mut context()).unwrap();
    assert_eq!(code(&at(-1), &placed), "invalid");
    assert_eq!(
        code(&at(::project::limits::MAX_TICK - 100), &placed),
        "invalid"
    );
    // A slot that is taken, or a scene that does not exist.
    let (in_slot, _) = after(
        f.state.clone(),
        &import_at(
            &f,
            Some(ClipTarget::Slot {
                scene: f.scene_free.clone(),
            }),
        ),
    );
    let slot = Some(ClipTarget::Slot {
        scene: f.scene_free.clone(),
    });
    assert_eq!(code(&import_at(&f, slot), &in_slot), "invalid");
    let mut ids = ::project::IdGenerator::new(9, 9);
    let ghost = Some(ClipTarget::Slot {
        scene: ::project::SceneId::generate(&mut ids),
    });
    assert_eq!(code(&import_at(&f, ghost), &f.state), "not_found");
}

#[test]
fn a_range_change_moves_the_whole_placements_and_undo_gives_their_lengths_back() {
    let f = fx();
    // Two whole placements: at 0 (the next one starts at 2880, 1.5 s later) and at 2880.
    let (state, first) = with_whole(&f, f.state.clone(), 0, 1);
    let (state, second) = with_whole(&f, state, 2880, 2);
    // A partial placement of the same clip, left alone.
    let (mut state, partial) = {
        let mut s = state;
        let mut ids = ::project::IdGenerator::new(1_690_000_000_000, 3);
        let id = PlacementId::generate(&mut ids);
        s.project
            .tracks
            .get_mut(&f.audio_track)
            .unwrap()
            .placements
            .insert(
                id.clone(),
                Placement {
                    clip: f.audio_clip.clone(),
                    start: Tick(100_000),
                    span: Span::Frames {
                        length: Frame::new(10_000),
                        offset: Frame::new(5_000),
                    },
                },
            );
        (s, id)
    };
    let before = state.clone();
    // Grow to the whole file (88 200 frames, 2 s): the first stops at the second's start
    // (1.5 s = 66 150 frames), the second grows whole, the partial one stays.
    let out = run(&range(&f, 0, 88_200), &mut state, &mut context()).unwrap();
    assert_eq!(
        span_of(&state, &f, &first),
        Span::Frames {
            length: Frame::new(66_150),
            offset: Frame::new(0)
        }
    );
    assert_eq!(
        span_of(&state, &f, &second),
        Span::Frames {
            length: Frame::new(88_200),
            offset: Frame::new(0)
        }
    );
    assert_eq!(
        span_of(&state, &f, &partial),
        span_of(&before, &f, &partial)
    );
    state.project.validate().unwrap();
    let Undo::Inverse(undo) = out.undo else {
        panic!()
    };
    run(&undo, &mut state, &mut context()).unwrap();
    assert_eq!(state.project, before.project);

    // Shrink: the whole ones follow; the partial one (offset 5 000) still starts inside.
    let mut shrunk = before.clone();
    run(&range(&f, 1_000, 20_000), &mut shrunk, &mut context()).unwrap();
    assert_eq!(
        span_of(&shrunk, &f, &first),
        Span::Frames {
            length: Frame::new(20_000),
            offset: Frame::new(0)
        }
    );
    // A partial placement may play past the new end of the clip: it repeats the clip (#473).
    assert_eq!(
        span_of(&shrunk, &f, &partial),
        span_of(&before, &f, &partial)
    );
    shrunk.project.validate().unwrap();
    // Its offset past the new length is refused.
    assert_eq!(code(&range(&f, 0, 5_000), &before), "invalid");
}

#[test]
fn range_gain_and_loop_refuse_what_the_project_would_not_hold() {
    let f = fx();
    let s = &f.state;
    // Empty, past the file (88 200 frames), overflowing.
    assert_eq!(code(&range(&f, 0, 0), s), "invalid");
    assert_eq!(code(&range(&f, 1, 88_200), s), "invalid");
    assert_eq!(code(&range(&f, u64::MAX, 2), s), "invalid");
    // A loop that would end past the new length.
    let (looped, _) = after(
        s.clone(),
        &c(clip::Command::AudioSetLoop {
            clip: f.audio_clip.clone(),
            looping: Some(FrameLoop {
                start: Frame::new(0),
                end: Frame::new(30_000),
            }),
        }),
    );
    assert_eq!(code(&range(&f, 0, 20_000), &looped), "invalid");
    run(&range(&f, 0, 30_000), &mut looped.clone(), &mut context()).unwrap();
    // A MIDI clip, an unknown clip.
    let midi = c(clip::Command::AudioSetGain {
        clip: f.clip.clone(),
        gain_db: Some(0.0),
    });
    assert_eq!(code(&midi, s), "invalid");
    let mut ids = ::project::IdGenerator::new(9, 9);
    let ghost = c(clip::Command::AudioSetLoop {
        clip: ::project::ClipId::generate(&mut ids),
        looping: None,
    });
    assert_eq!(code(&ghost, s), "not_found");
    let midi_range = c(clip::Command::AudioSetRange {
        clip: f.clip.clone(),
        start: Frame::new(0),
        frames: Frame::new(1),
        placements: None,
    });
    assert_eq!(code(&midi_range, s), "invalid");
    // Gains outside -96..+6 dB, not a number.
    for g in [6.01, -96.5, f64::NAN, f64::INFINITY] {
        let gain = c(clip::Command::AudioSetGain {
            clip: f.audio_clip.clone(),
            gain_db: Some(g),
        });
        assert_eq!(code(&gain, s), "invalid", "{g}");
    }
    // Loops: empty, reversed, past the clip's 44 100 frames.
    for (a, b) in [(10, 10), (20, 10), (0, 44_101)] {
        let l = c(clip::Command::AudioSetLoop {
            clip: f.audio_clip.clone(),
            looping: Some(FrameLoop {
                start: Frame::new(a),
                end: Frame::new(b),
            }),
        });
        assert_eq!(code(&l, s), "invalid", "{a}..{b}");
    }
}

#[test]
fn silence_is_a_null_gain_kept_through_undo_and_the_folder() {
    let f = fx();
    let mut state = f.state.clone();
    let out = run(
        &c(clip::Command::AudioSetGain {
            clip: f.audio_clip.clone(),
            gain_db: None,
        }),
        &mut state,
        &mut context(),
    )
    .unwrap();
    let ClipContent::Audio { gain_db, .. } = audio(&state, &f) else {
        panic!()
    };
    assert_eq!(gain_db, f64::NEG_INFINITY);
    // A value already in place changes nothing.
    let again = run(
        &c(clip::Command::AudioSetGain {
            clip: f.audio_clip.clone(),
            gain_db: None,
        }),
        &mut state.clone(),
        &mut context(),
    )
    .unwrap();
    assert!(matches!(again.undo, Undo::None) && again.events.is_empty());
    // Through the folder: written and read back the same.
    let mut fs = MemFs::new();
    save(
        &mut fs,
        &state.project,
        &mut SaveState::new(),
        None,
        &SaveOptions::default(),
    )
    .unwrap();
    let read = open(&fs, &OpenOptions::default()).unwrap();
    assert_eq!(
        read.project.tracks[&f.audio_track].clips,
        state.project.tracks[&f.audio_track].clips
    );
    // Undo gives 0 dB back.
    let Undo::Inverse(undo) = out.undo else {
        panic!()
    };
    run(&undo, &mut state, &mut context()).unwrap();
    assert_eq!(state.project, f.state.project);
}

#[test]
fn a_tempo_that_would_push_an_audio_placement_past_the_timeline_is_refused() {
    let f = fx();
    // One second of audio near the end of the timeline: 1 920 ticks at 120 BPM, 3 840 at
    // 240 (a faster tempo puts more ticks in a second), 960 at 60.
    let start = ::project::limits::MAX_TICK - 3_000;
    let (state, _) = with_whole(&f, f.state.clone(), start, 4);
    let tempo = |bpm: f64| Command::Transport(transport::Command::SetTempo { bpm });
    assert_eq!(code(&tempo(240.0), &state), "invalid");
    let mut slower = state.clone();
    run(&tempo(60.0), &mut slower, &mut context()).unwrap();
    slower.project.validate().unwrap();
    // Without audio placements, any tempo of the range goes.
    run(&tempo(999.0), &mut f.state.clone(), &mut context()).unwrap();
}

#[test]
fn an_audio_clip_moves_in_the_grid_of_audio_tracks_only() {
    let f = fx();
    let mv = |track: Option<&::project::TrackId>, slot: Option<&::project::SceneId>| {
        c(clip::Command::Move {
            clip: f.audio_clip.clone(),
            track: track.cloned(),
            slot: slot.cloned(),
        })
    };
    let mut state = f.state.clone();
    let out = run(&mv(None, Some(&f.scene_free)), &mut state, &mut context()).unwrap();
    assert_eq!(
        state.project.tracks[&f.audio_track].clips[&f.audio_clip].slot,
        Some(f.scene_free.clone())
    );
    state.project.validate().unwrap();
    let Undo::Inverse(undo) = out.undo else {
        panic!()
    };
    run(&undo, &mut state, &mut context()).unwrap();
    assert_eq!(state.project, f.state.project);
    assert_eq!(
        code(&mv(Some(&f.track), Some(&f.scene_free)), &f.state),
        "invalid"
    );
}

/// The fixture's project saved to a folder, with or without the audio clip's file
/// (`blobs/<hash>`), placed in a slot and in the arrangement with its range, gain and loop
/// set, then opened through `project.open`.
fn opened(with_blob: bool) -> (ClipFx, State, commands::Context, State) {
    let f = fx();
    let mut state = f.state.clone();
    for command in [
        mv_to(&f),
        range(&f, 4_410, 39_690),
        c(clip::Command::AudioSetGain {
            clip: f.audio_clip.clone(),
            gain_db: Some(-3.25),
        }),
        c(clip::Command::AudioSetLoop {
            clip: f.audio_clip.clone(),
            looping: Some(FrameLoop {
                start: Frame::new(441),
                end: Frame::new(22_050),
            }),
        }),
    ] {
        run(&command, &mut state, &mut context()).unwrap();
    }
    let (state, _) = with_whole_of(&f, state);
    let mut fs = MemFs::new();
    save(
        &mut fs,
        &state.project,
        &mut SaveState::new(),
        None,
        &SaveOptions::default(),
    )
    .unwrap();
    let ClipContent::Audio { media, .. } = audio(&state, &f) else {
        panic!()
    };
    if with_blob {
        fs.write(&format!("blobs/{}", media.hash), b"RIFF").unwrap();
    }
    let mut ctx = bare_context();
    ctx.folder = Some(Folder::new("song", fs));
    let mut reopened = commands::testing::fixture().state;
    run(
        &Command::Project(commands::project::Command::Open {
            folder: "song".into(),
        }),
        &mut reopened,
        &mut ctx,
    )
    .unwrap();
    (f, state, ctx, reopened)
}

fn mv_to(f: &ClipFx) -> Command {
    c(clip::Command::Move {
        clip: f.audio_clip.clone(),
        track: None,
        slot: Some(f.scene_free.clone()),
    })
}

/// A placement playing the clip's 39 690 frames whole from tick 0.
fn with_whole_of(f: &ClipFx, mut state: State) -> (State, PlacementId) {
    let mut ids = ::project::IdGenerator::new(1_690_000_000_000, 5);
    let id = PlacementId::generate(&mut ids);
    state
        .project
        .tracks
        .get_mut(&f.audio_track)
        .unwrap()
        .placements
        .insert(
            id.clone(),
            Placement {
                clip: f.audio_clip.clone(),
                start: Tick(0),
                span: Span::Frames {
                    length: Frame::new(39_690),
                    offset: Frame::new(0),
                },
            },
        );
    state.project.validate().unwrap();
    (state, id)
}

fn clip_get(f: &ClipFx, state: &mut State, ctx: &mut commands::Context) -> clip::Output {
    match run(
        &c(clip::Command::Get {
            clip: f.audio_clip.clone(),
            offset: None,
            limit: None,
        }),
        state,
        ctx,
    )
    .unwrap()
    .output
    {
        Output::Clip(out) => out,
        other => panic!("{other:?}"),
    }
}

#[test]
fn an_audio_clip_in_a_slot_and_a_placement_reads_back_the_same_from_its_folder() {
    let (f, saved, mut ctx, mut reopened) = opened(true);
    same_audio(&f, &reopened, &saved);
    let clip::Output::Get { summary, .. } = clip_get(&f, &mut reopened, &mut ctx) else {
        panic!()
    };
    assert_eq!(summary.media_missing, Some(false));
    assert_eq!(summary.slot, Some(f.scene_free.clone()));
}

#[test]
fn a_clip_whose_file_is_missing_opens_marked_and_stays_editable() {
    let (f, saved, mut ctx, mut reopened) = opened(false);
    // The project is valid and the same; the clip is marked "média absent".
    same_audio(&f, &reopened, &saved);
    reopened.project.validate().unwrap();
    let clip::Output::Get { summary, .. } = clip_get(&f, &mut reopened, &mut ctx) else {
        panic!()
    };
    assert_eq!(summary.media_missing, Some(true));
    // Through `track.get` too; a MIDI clip has no such state.
    let track = run(
        &Command::Track(commands::track::Command::Get {
            track: f.audio_track.clone(),
            offset: None,
            limit: None,
        }),
        &mut reopened,
        &mut ctx,
    )
    .unwrap();
    let Output::Track(commands::track::Output::Get { clips, .. }) = track.output else {
        panic!()
    };
    assert_eq!(clips[0].media_missing, Some(true));
    // The commands still work on it.
    run(
        &range(&f, 0, 30_000),
        &mut reopened.clone(),
        &mut ctx.clone(),
    )
    .unwrap();
    // Staging the file again (the page wrote its blob first) clears the mark.
    let ClipContent::Audio { media, .. } = audio(&reopened, &f) else {
        panic!()
    };
    ctx.missing_media.remove(&media.hash);
    let clip::Output::Get { summary, .. } = clip_get(&f, &mut reopened, &mut ctx) else {
        panic!()
    };
    assert_eq!(summary.media_missing, Some(false));
    // A new project forgets the old folder's state.
    run(
        &Command::Project(commands::project::Command::New { name: None }),
        &mut reopened,
        &mut ctx,
    )
    .unwrap();
    assert!(ctx.missing_media.is_empty());
}

/// The audio track's clips and placements are the same (opening a folder gives an
/// uncoloured track the hue of its position, which is not what is compared here).
fn same_audio(f: &ClipFx, a: &State, b: &State) {
    let (a, b) = (
        &a.project.tracks[&f.audio_track],
        &b.project.tracks[&f.audio_track],
    );
    assert_eq!((&a.clips, &a.placements), (&b.clips, &b.placements));
}

#[test]
fn the_undo_lengths_of_a_range_change_are_checked_like_any_placement() {
    let f = fx();
    // One second (1 920 ticks) of audio 3 000 ticks before the end of the timeline.
    let (state, whole) = with_whole(&f, f.state.clone(), ::project::limits::MAX_TICK - 3_000, 6);
    let (partial_state, partial) = with_audio_placement(&f);
    let with = |placements: Vec<clip::PlacementFrames>| {
        c(clip::Command::AudioSetRange {
            clip: f.audio_clip.clone(),
            start: Frame::new(0),
            frames: Frame::new(44_100),
            placements: Some(placements),
        })
    };
    let item = |id: &PlacementId, n: u64| clip::PlacementFrames {
        placement: id.clone(),
        length: Frame::new(n),
    };
    // An end past 2^40 ticks at the tempo, no frame, more than 2^40 frames, listed twice.
    let past = ::project::limits::MAX_FRAME;
    // Two seconds are 3 840 ticks: past the end.
    assert_eq!(code(&with(vec![item(&whole, 88_200)]), &state), "invalid");
    assert_eq!(code(&with(vec![item(&whole, 0)]), &state), "invalid");
    assert_eq!(code(&with(vec![item(&whole, past + 1)]), &state), "invalid");
    assert_eq!(
        code(&with(vec![item(&whole, 10), item(&whole, 20)]), &state),
        "invalid"
    );
    // A placement of no clip of this one.
    assert_eq!(code(&with(vec![item(&partial, 10)]), &state), "not_found");
    // A length that fits is taken as is.
    let mut taken = partial_state.clone();
    run(&with(vec![item(&partial, 7)]), &mut taken, &mut context()).unwrap();
    assert_eq!(
        span_of(&taken, &f, &partial),
        Span::Frames {
            length: Frame::new(7),
            offset: Frame::new(11_025)
        }
    );
}

#[test]
fn a_gain_beyond_the_range_stays_restorable_but_is_not_edited() {
    let f = fx();
    // A file may hold +12 dB: validation only asks for a number.
    let mut state = f.state.clone();
    if let ClipContent::Audio { gain_db, .. } = &mut state
        .project
        .tracks
        .get_mut(&f.audio_track)
        .unwrap()
        .clips
        .get_mut(&f.audio_clip)
        .unwrap()
        .content
    {
        *gain_db = 12.0;
    }
    state.project.validate().unwrap();
    let gain = c(clip::Command::AudioSetGain {
        clip: f.audio_clip.clone(),
        gain_db: Some(0.0),
    });
    assert_eq!(code(&gain, &state), "invalid");
    // Deleting the clip and undoing brings it back with its gain.
    let before = state.clone();
    let out = run(
        &c(clip::Command::Delete {
            clip: f.audio_clip.clone(),
        }),
        &mut state,
        &mut context(),
    )
    .unwrap();
    let Undo::Inverse(undo) = out.undo else {
        panic!()
    };
    run(&undo, &mut state, &mut context()).unwrap();
    assert_eq!(state.project, before.project);
}

#[test]
fn every_audio_command_refuses_a_midi_clip_and_an_unknown_clip() {
    let f = fx();
    let mut ids = ::project::IdGenerator::new(9, 9);
    let ghost = ::project::ClipId::generate(&mut ids);
    for clip in [f.clip.clone(), ghost.clone()] {
        let expected = if clip == ghost {
            "not_found"
        } else {
            "invalid"
        };
        for command in [
            clip::Command::AudioSetRange {
                clip: clip.clone(),
                start: Frame::new(0),
                frames: Frame::new(1),
                placements: None,
            },
            clip::Command::AudioSetGain {
                clip: clip.clone(),
                gain_db: Some(0.0),
            },
            clip::Command::AudioSetLoop {
                clip: clip.clone(),
                looping: None,
            },
        ] {
            assert_eq!(code(&c(command), &f.state), expected);
        }
    }
}
