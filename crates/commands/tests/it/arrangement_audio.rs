//! AU-6: audio placements under the arrangement commands. They follow the overlap rule of
//! the MIDI ones ("the last one put down wins"), cut in whole frames of their clip's file:
//! move, resize (clamped to the clip), split, duplicate (an independent copy is a new audio
//! clip of the same file), repeat, move to another audio track, and the time edits. Every
//! outcome stays valid, overlap free, inside its clip (`offset + length <= frames`), and is
//! undone exactly, through the history too (undo, then redo).

use crate::arrangement_examples::{self, ArFx, a, fx};
use ::project::{
    ClipContent, ClipId, Frame, FrameLoop, IdGenerator, Placement, PlacementId, SampleRate, Span,
    Tempo, Tick, TrackId,
};
use commands::arrangement::{self, Edge, TrackMove};
use commands::testing::{check_refusal, context};
use commands::{Command, CommandError, Output, State, Undo, history, run};

/// A context whose identifiers differ from every other call's.
fn fresh() -> commands::Context {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    commands::Context::new(
        commands::Author::User,
        IdGenerator::new(
            1_695_000_000_000 + n,
            0x00AB_0000_0000_0000_0000 + u128::from(n),
        ),
    )
}

/// Runs `command`, checks the project stays valid and its inverse gives back the state
/// before; returns the output.
fn ok(state: &mut State, command: arrangement::Command) -> arrangement::Output {
    let before = state.project.clone();
    let mut ctx = fresh();
    let outcome =
        run(&a(command.clone()), state, &mut ctx).unwrap_or_else(|e| panic!("{command:?}: {e:?}"));
    state
        .project
        .validate()
        .unwrap_or_else(|v| panic!("{command:?}: {v:?}"));
    if let Undo::Inverse(inverse) = &outcome.undo {
        let mut undone = state.clone();
        inverse.apply(&mut undone, &mut fresh()).unwrap();
        assert_eq!(undone.project, before, "{command:?}: the undo is exact");
    }
    let Output::Arrangement(output) = outcome.output else {
        panic!("not an arrangement output")
    };
    output
}

fn refused(state: &State, command: arrangement::Command) -> CommandError {
    check_refusal(&a(command), state, &context()).unwrap()
}

/// `(start, length, offset)` of every audio placement of the track, by start.
fn frames(state: &State, track: &TrackId) -> Vec<(i64, u64, u64)> {
    let mut out: Vec<_> = state.project.tracks[track]
        .placements
        .values()
        .map(|p| match p.span {
            Span::Frames { length, offset } => (p.start.get(), length.get(), offset.get()),
            Span::Ticks { .. } => panic!("a tick placement in an audio track"),
        })
        .collect();
    out.sort();
    out
}

fn frames_of(state: &State, track: &TrackId, id: &PlacementId) -> (i64, u64, u64) {
    let p = &state.project.tracks[track].placements[id];
    match p.span {
        Span::Frames { length, offset } => (p.start.get(), length.get(), offset.get()),
        Span::Ticks { .. } => panic!("ticks"),
    }
}

/// The fixture with one placement of its audio clip (one second, 44 100 frames at 44.1 kHz,
/// 1920 ticks at 120 bpm) from `start`.
fn with_second(f: &ArFx, start: i64) -> (State, PlacementId) {
    let mut state = f.state.clone();
    let id = PlacementId::generate(&mut IdGenerator::new(
        1_690_500_000_000 + start.unsigned_abs(),
        0x00AC_0000_0000_0000_0000,
    ));
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

/// Every audio placement of the project ends inside its clip and no two placements of a
/// track overlap in ticks.
fn sound(state: &State) {
    let tempo = state.project.tempo();
    for (track_id, track) in &state.project.tracks {
        let mut spans = Vec::new();
        for p in track.placements.values() {
            match (p.span, &track.clips[&p.clip].content) {
                (Span::Frames { length, offset }, ClipContent::Audio { frames, media, .. }) => {
                    assert!(
                        offset.get() + length.get() <= frames.get(),
                        "{track_id}: {p:?} ends past its clip"
                    );
                    let ticks = tempo
                        .frames_to_ticks(length, media.sample_rate)
                        .get()
                        .max(1);
                    spans.push((p.start.get(), p.start.get() + ticks));
                }
                (Span::Ticks { length, .. }, ClipContent::Midi { .. }) => {
                    spans.push((p.start.get(), p.start.get() + length.get()));
                }
                other => panic!("{other:?}"),
            }
        }
        spans.sort();
        for pair in spans.windows(2) {
            assert!(pair[0].1 <= pair[1].0, "{track_id}: overlap in {spans:?}");
        }
    }
}

#[test]
fn a_move_keeps_the_frames_and_carves_the_neighbour_it_lands_on() {
    let f = fx();
    let (mut state, first) = with_second(&f, 0);
    let (_, second) = {
        let (s, id) = with_second(&f, 3840);
        let p = s.project.tracks[&f.audio_track].placements[&id].clone();
        state
            .project
            .tracks
            .get_mut(&f.audio_track)
            .unwrap()
            .placements
            .insert(id.clone(), p);
        (s, id)
    };
    // The second one moves half a second earlier, onto the end of the first.
    let out = ok(
        &mut state,
        arrangement::Command::Move {
            placements: vec![second.clone()],
            by: Tick(-2880),
        },
    );
    assert!(matches!(out, arrangement::Output::Move { overwritten: 1 }));
    assert_eq!(frames_of(&state, &f.audio_track, &second), (960, 44_100, 0));
    assert_eq!(frames_of(&state, &f.audio_track, &first), (0, 22_050, 0));
    assert_eq!(
        frames(&state, &f.audio_track),
        [(0, 22_050, 0), (960, 44_100, 0)]
    );
    sound(&state);
}

#[test]
fn resizing_an_audio_edge_works_in_frames_and_stops_at_the_clip() {
    let f = fx();
    // 22 050 frames from frame 11 025, from tick 3840 (half a second, 960 ticks).
    let (placed, id) = arrangement_examples::with_audio_placement(&f);
    let resize = |edge: Edge, to: i64| arrangement::Command::Resize {
        placement: id.clone(),
        edge,
        to: Tick(to),
    };
    let at = |state: &State| frames_of(state, &f.audio_track, &id);
    // The end: shorter, then longer up to the clip's last frame (and no further).
    let mut s = placed.clone();
    ok(&mut s, resize(Edge::End, 3840 + 480));
    assert_eq!(at(&s), (3840, 11_025, 11_025));
    let mut s = placed.clone();
    ok(&mut s, resize(Edge::End, 3840 + 10_000));
    assert_eq!(at(&s), (3840, 33_075, 11_025), "up to the clip's end");
    // The start: later (the content does not slide), then earlier down to frame 0.
    let mut s = placed.clone();
    ok(&mut s, resize(Edge::Start, 3840 + 480));
    assert_eq!(at(&s), (4320, 11_025, 22_050));
    let mut s = placed.clone();
    ok(&mut s, resize(Edge::Start, 3840 - 240));
    assert_eq!(at(&s), (3600, 27_562, 5_513));
    let mut s = placed.clone();
    ok(&mut s, resize(Edge::Start, 0));
    assert_eq!(at(&s), (3360, 33_075, 0), "down to the clip's first frame");
    // An edge moved where it is, or a start pulled left of a placement already at the clip's
    // first frame, changes nothing (no history entry).
    for (state, command) in [
        (&placed, resize(Edge::End, 4800)),
        (&placed, resize(Edge::Start, 3840)),
    ] {
        let mut s = state.clone();
        let outcome = run(&a(command), &mut s, &mut fresh()).unwrap();
        assert!(matches!(outcome.undo, Undo::None));
        assert_eq!(s.project, state.project);
    }
    let mut at_zero = placed.clone();
    ok(&mut at_zero, resize(Edge::Start, 0));
    let before = at_zero.clone();
    let outcome = run(&a(resize(Edge::Start, 1000)), &mut at_zero, &mut fresh()).unwrap();
    assert!(matches!(outcome.undo, Undo::None));
    assert_eq!(
        at_zero.project, before.project,
        "the start stays at the clip's first frame"
    );
    // 27 560 frames are 1199.9 ticks, rounded to 1200: an end resized to where it is keeps them.
    let mut odd = placed.clone();
    if let Some(p) = odd
        .project
        .tracks
        .get_mut(&f.audio_track)
        .unwrap()
        .placements
        .get_mut(&id)
    {
        p.span = Span::Frames {
            length: Frame::new(27_560),
            offset: Frame::new(0),
        };
    }
    let before = odd.clone();
    let outcome = run(&a(resize(Edge::End, 3840 + 1200)), &mut odd, &mut fresh()).unwrap();
    assert!(matches!(outcome.undo, Undo::None));
    assert_eq!(odd.project, before.project);
    // The end stays after the start, the start before the end.
    for command in [resize(Edge::End, 3840), resize(Edge::Start, 4800)] {
        assert!(matches!(
            refused(&placed, command),
            CommandError::Invalid { .. }
        ));
    }
}

#[test]
fn a_split_continues_the_frames_and_goes_round_the_clips_loop() {
    let f = fx();
    let (placed, id) = with_second(&f, 0);
    let mut s = placed.clone();
    let out = ok(
        &mut s,
        arrangement::Command::Split {
            placement: id.clone(),
            at: Tick(480),
        },
    );
    let arrangement::Output::Split { placement: right } = out else {
        panic!()
    };
    assert_eq!(frames_of(&s, &f.audio_track, &id), (0, 11_025, 0));
    assert_eq!(frames_of(&s, &f.audio_track, &right), (480, 33_075, 11_025));
    // With a loop [0, 4410) the placement plays the loop ten times: a split after the first
    // pass resumes inside the loop.
    let mut looped = placed.clone();
    if let ClipContent::Audio { looping, .. } = &mut looped
        .project
        .tracks
        .get_mut(&f.audio_track)
        .unwrap()
        .clips
        .get_mut(&f.audio_clip)
        .unwrap()
        .content
    {
        *looping = Some(FrameLoop {
            start: Frame::new(0),
            end: Frame::new(4_410),
        });
    }
    let out = ok(
        &mut looped,
        arrangement::Command::Split {
            placement: id.clone(),
            at: Tick(480),
        },
    );
    let arrangement::Output::Split { placement: right } = out else {
        panic!()
    };
    // 11 025 frames in: two passes and a half of the 4 410-frame loop.
    assert_eq!(
        frames_of(&looped, &f.audio_track, &right),
        (480, 33_075, 11_025 % 4_410)
    );
    sound(&looped);
}

#[test]
fn a_split_or_cut_too_close_to_an_edge_is_refused_and_never_makes_an_empty_part() {
    // At the fastest tempo and the lowest rate a tick holds less than a frame.
    let f = fx();
    let (mut s, id) = with_second(&f, 0);
    s.project.tempo[0].bpm = Tempo::clamped(Tempo::MAX);
    let clip = s
        .project
        .tracks
        .get_mut(&f.audio_track)
        .unwrap()
        .clips
        .get_mut(&f.audio_clip)
        .unwrap();
    if let ClipContent::Audio { media, .. } = &mut clip.content {
        media.sample_rate = SampleRate::new(8_000).unwrap();
    }
    s.project.validate().unwrap();
    let error = refused(
        &s,
        arrangement::Command::Split {
            placement: id.clone(),
            at: Tick(1),
        },
    );
    assert!(matches!(error, CommandError::Invalid { .. }), "{error:?}");
    // Every tick of the first few: refused or two non-empty parts, never a panic.
    for at in 1..40 {
        let mut t = s.clone();
        if let Ok(outcome) = run(
            &a(arrangement::Command::Split {
                placement: id.clone(),
                at: Tick(at),
            }),
            &mut t,
            &mut fresh(),
        ) {
            t.project.validate().unwrap();
            sound(&t);
            assert!(matches!(outcome.undo, Undo::Inverse(_)));
        }
    }
}

#[test]
fn an_independent_copy_is_a_new_audio_clip_of_the_same_file_and_redo_brings_it_back() {
    let f = fx();
    let (placed, id) = arrangement_examples::with_audio_placement(&f);
    let mut state = placed.clone();
    let mut ctx = fresh();
    let outcome = run(
        &a(arrangement::Command::Duplicate {
            placements: vec![id.clone()],
            linked: false,
            by: None,
        }),
        &mut state,
        &mut ctx,
    )
    .unwrap();
    let Output::Arrangement(arrangement::Output::Duplicate {
        placements, clips, ..
    }) = outcome.output
    else {
        panic!()
    };
    let track = &state.project.tracks[&f.audio_track];
    let copy = &track.clips[&clips[0]];
    assert_eq!(copy.content, track.clips[&f.audio_clip].content);
    assert_eq!(copy.slot, None);
    // Right after the original: half a second later, the same frames.
    assert_eq!(
        frames_of(&state, &f.audio_track, &placements[0]),
        (4800, 22_050, 11_025)
    );
    // Through the history: undo removes the copy and its clip, redo brings both back.
    let after = state.project.clone();
    let mut state = placed.clone();
    let mut ctx = fresh();
    run(
        &a(arrangement::Command::Duplicate {
            placements: vec![id.clone()],
            linked: false,
            by: None,
        }),
        &mut state,
        &mut ctx,
    )
    .unwrap();
    let redone_target = state.project.clone();
    run(
        &Command::History(history::Command::Undo {}),
        &mut state,
        &mut ctx,
    )
    .unwrap();
    assert_eq!(state.project, placed.project);
    run(
        &Command::History(history::Command::Redo {}),
        &mut state,
        &mut ctx,
    )
    .unwrap();
    assert_eq!(state.project, redone_target);
    assert_eq!(
        state.project.tracks[&f.audio_track].clips.len(),
        after.tracks[&f.audio_track].clips.len()
    );
    // Linked: a new placement of the same clip.
    let mut linked = placed;
    let out = ok(
        &mut linked,
        arrangement::Command::Duplicate {
            placements: vec![id],
            linked: true,
            by: Some(Tick(3840)),
        },
    );
    let arrangement::Output::Duplicate {
        placements, clips, ..
    } = out
    else {
        panic!()
    };
    assert!(clips.is_empty());
    let p = &linked.project.tracks[&f.audio_track].placements[&placements[0]];
    assert_eq!(p.clip, f.audio_clip);
}

#[test]
fn an_audio_placement_moves_to_another_audio_track_only() {
    let f = fx();
    let (placed, id) = arrangement_examples::with_audio_placement(&f);
    let mut state = placed.clone();
    let out = commands::run(
        &Command::Track(commands::track::Command::Create {
            kind: ::project::TrackKind::Audio,
            name: None,
            color: None,
            instrument: None,
        }),
        &mut state,
        &mut fresh(),
    );
    let Ok(outcome) = out else { panic!("{out:?}") };
    let Output::Track(commands::track::Output::Create { track: other, .. }) = outcome.output else {
        panic!()
    };
    let before = state.clone();
    let out = ok(
        &mut state,
        arrangement::Command::MoveToTrack {
            moves: vec![TrackMove {
                placement: id.clone(),
                track: other.clone(),
            }],
            by: Some(Tick(960)),
        },
    );
    let arrangement::Output::MoveToTrack { placements, .. } = out else {
        panic!()
    };
    // The clip went with it (it had no other use and no slot), as a copy in the target.
    assert!(state.project.tracks[&f.audio_track].placements.is_empty());
    assert!(
        !state.project.tracks[&f.audio_track]
            .clips
            .contains_key(&f.audio_clip)
    );
    assert_eq!(
        frames_of(&state, &other, &placements[0]),
        (4800, 22_050, 11_025)
    );
    sound(&state);
    // Through the history: undo brings the clip back with its identifier, redo again.
    let mut s = before.clone();
    let mut ctx = fresh();
    let mv = a(arrangement::Command::MoveToTrack {
        moves: vec![TrackMove {
            placement: id.clone(),
            track: other.clone(),
        }],
        by: None,
    });
    run(&mv, &mut s, &mut ctx).unwrap();
    let moved = s.project.clone();
    run(
        &Command::History(history::Command::Undo {}),
        &mut s,
        &mut ctx,
    )
    .unwrap();
    assert_eq!(s.project, before.project);
    run(
        &Command::History(history::Command::Redo {}),
        &mut s,
        &mut ctx,
    )
    .unwrap();
    assert_eq!(s.project, moved);
    // To a MIDI track, and a MIDI placement to the audio track: refused.
    for command in [
        arrangement::Command::MoveToTrack {
            moves: vec![TrackMove {
                placement: id.clone(),
                track: f.track.clone(),
            }],
            by: None,
        },
        arrangement::Command::MoveToTrack {
            moves: vec![TrackMove {
                placement: f.a.clone(),
                track: f.audio_track.clone(),
            }],
            by: None,
        },
    ] {
        assert!(matches!(
            refused(&before, command),
            CommandError::Invalid { .. }
        ));
    }
}

#[test]
fn a_grid_audio_clip_is_placed_whole_as_a_copy_or_linked() {
    let f = fx();
    let mut state = f.state.clone();
    state
        .project
        .tracks
        .get_mut(&f.audio_track)
        .unwrap()
        .clips
        .get_mut(&f.audio_clip)
        .unwrap()
        .slot = Some(f.scene_free.clone());
    for linked in [false, true] {
        let mut s = state.clone();
        let out = ok(
            &mut s,
            arrangement::Command::PlaceFromSlot {
                clip: f.audio_clip.clone(),
                start: Tick(3840),
                linked,
            },
        );
        let arrangement::Output::PlaceFromSlot {
            placement, clip, ..
        } = out
        else {
            panic!()
        };
        assert_eq!(clip.is_none(), linked);
        let p = &s.project.tracks[&f.audio_track].placements[&placement];
        assert_eq!(p.clip, clip.unwrap_or_else(|| f.audio_clip.clone()));
        assert_eq!(frames_of(&s, &f.audio_track, &placement), (3840, 44_100, 0));
    }
}

/// A deterministic generator for the sweep.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self, n: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) % n
    }
}

#[test]
fn random_audio_edits_stay_inside_their_clips_overlap_free_and_undo_through_the_history() {
    for (seed, bpm, rate, looped) in [
        (5u64, 120.0, 44_100u32, false),
        (11, 97.3, 48_000, false),
        (23, 173.0, 8_000, false),
        (31, 61.0, 96_000, false),
        // A looped clip: the cuts go round its loop.
        (41, 133.0, 44_100, true),
    ] {
        let f = fx();
        let (mut state, _) = with_second(&f, 0);
        state.project.tempo[0].bpm = Tempo::clamped(bpm);
        if let ClipContent::Audio { media, .. } = &mut state
            .project
            .tracks
            .get_mut(&f.audio_track)
            .unwrap()
            .clips
            .get_mut(&f.audio_clip)
            .unwrap()
            .content
        {
            media.sample_rate = SampleRate::new(rate).unwrap();
        }
        if looped
            && let ClipContent::Audio { looping, .. } = &mut state
                .project
                .tracks
                .get_mut(&f.audio_track)
                .unwrap()
                .clips
                .get_mut(&f.audio_clip)
                .unwrap()
                .content
        {
            *looping = Some(FrameLoop {
                start: Frame::new(5_000),
                end: Frame::new(17_000),
            });
        }
        // A second audio track, for `move_to_track`.
        let Output::Track(commands::track::Output::Create { track: other, .. }) = run(
            &Command::Track(commands::track::Command::Create {
                kind: ::project::TrackKind::Audio,
                name: None,
                color: None,
                instrument: None,
            }),
            &mut state,
            &mut fresh(),
        )
        .unwrap()
        .output
        else {
            panic!("no track")
        };
        state.project.validate().unwrap();
        let start = state.clone();
        let mut ctx = fresh();
        let mut rng = Lcg(seed);
        let mut done = 0usize;
        for step in 0..250 {
            let ids: Vec<PlacementId> = state.project.tracks[&f.audio_track]
                .placements
                .keys()
                .cloned()
                .collect();
            let clips: Vec<ClipId> = state.project.tracks[&f.audio_track]
                .clips
                .keys()
                .cloned()
                .collect();
            if clips.is_empty() {
                break;
            }
            let pick = |rng: &mut Lcg| ids[rng.next(ids.len() as u64) as usize].clone();
            let tick = |rng: &mut Lcg| Tick(rng.next(9_000) as i64);
            // An empty track takes a placement first (a time edit or a move may empty it).
            let roll = if ids.is_empty() { 0 } else { rng.next(10) };
            // Now and then a command that changes what the spans measure (#703): the tempo, or
            // the clip's range. They refuse to leave a placement outside its clip or overlapping.
            let measure = !ids.is_empty() && rng.next(8) == 0;
            let command: Command = if measure {
                if rng.next(2) == 0 {
                    Command::Transport(commands::transport::Command::SetTempo {
                        bpm: 60.0 + rng.next(140) as f64,
                    })
                } else {
                    Command::Clip(commands::clip::Command::AudioSetRange {
                        clip: f.audio_clip.clone(),
                        start: Frame::new(0),
                        frames: Frame::new(20_000 + rng.next(24_100)),
                        placements: None,
                    })
                }
            } else {
                a(match roll {
                    9 if ids.len() > 3 => arrangement::Command::MoveToTrack {
                        moves: vec![TrackMove {
                            placement: pick(&mut rng),
                            track: other.clone(),
                        }],
                        by: Some(Tick(rng.next(4_000) as i64 - 2_000)),
                    },
                    0 => arrangement::Command::Place {
                        clip: clips[rng.next(clips.len() as u64) as usize].clone(),
                        start: tick(&mut rng),
                        length: None,
                        offset: None,
                    },
                    1 => arrangement::Command::Move {
                        placements: vec![pick(&mut rng)],
                        by: Tick(rng.next(4_000) as i64 - 2_000),
                    },
                    2 => arrangement::Command::Resize {
                        placement: pick(&mut rng),
                        edge: if rng.next(2) == 0 {
                            Edge::Start
                        } else {
                            Edge::End
                        },
                        to: tick(&mut rng),
                    },
                    3 => arrangement::Command::Split {
                        placement: pick(&mut rng),
                        at: tick(&mut rng),
                    },
                    4 if ids.len() > 1 => arrangement::Command::Delete {
                        placements: vec![pick(&mut rng)],
                    },
                    5 => arrangement::Command::Duplicate {
                        placements: vec![pick(&mut rng)],
                        linked: rng.next(2) == 0,
                        by: Some(Tick(rng.next(6_000) as i64 - 1_500)),
                    },
                    6 => arrangement::Command::Repeat {
                        placement: pick(&mut rng),
                        until: Tick(rng.next(12_000) as i64 + 1),
                    },
                    7 => arrangement::Command::InsertTime {
                        at: tick(&mut rng),
                        length: Tick(1 + rng.next(2_000) as i64),
                    },
                    _ => {
                        let from = rng.next(6_000) as i64;
                        arrangement::Command::DeleteTime {
                            from: Tick(from),
                            to: Tick(from + 1 + rng.next(2_000) as i64),
                        }
                    }
                })
            };
            let before = state.clone();
            match run(&command, &mut state, &mut ctx) {
                Ok(outcome) => {
                    state
                        .project
                        .validate()
                        .unwrap_or_else(|v| panic!("{bpm} {rate} step {step} {command:?}: {v:?}"));
                    sound(&state);
                    if let Undo::Inverse(inverse) = outcome.undo {
                        let mut undone = state.clone();
                        inverse
                            .apply(&mut undone, &mut fresh())
                            .unwrap_or_else(|e| {
                                panic!("{bpm} {rate} step {step} {command:?}: undo {e:?}")
                            });
                        assert_eq!(
                            undone.project, before.project,
                            "{bpm} {rate} step {step} {command:?}"
                        );
                        done += 1;
                    }
                    // Deleting any audio placement can be undone (#703).
                    let held: Vec<PlacementId> = state.project.tracks[&f.audio_track]
                        .placements
                        .keys()
                        .cloned()
                        .collect();
                    if let Some(gone) = held.get(rng.next(held.len().max(1) as u64) as usize) {
                        let mut probe = state.clone();
                        let deleted = run(
                            &a(arrangement::Command::Delete {
                                placements: vec![gone.clone()],
                            }),
                            &mut probe,
                            &mut fresh(),
                        )
                        .unwrap();
                        let Undo::Inverse(back) = deleted.undo else {
                            panic!()
                        };
                        back.apply(&mut probe, &mut fresh()).unwrap_or_else(|e| {
                            panic!("{bpm} {rate} step {step}: undo of a delete {e:?}")
                        });
                        assert_eq!(probe.project, state.project);
                    }
                }
                Err(_) => assert_eq!(state, before, "step {step}: a refusal changes nothing"),
            }
        }
        assert!(
            done > 50,
            "{bpm} {rate}: the sweep changes the project ({done})"
        );
        // Back to the start through the history, then forward again.
        let end = state.project.clone();
        for _ in 0..done {
            run(
                &Command::History(history::Command::Undo {}),
                &mut state,
                &mut ctx,
            )
            .unwrap();
        }
        assert_eq!(state.project, start.project, "{bpm} {rate}");
        for _ in 0..done {
            run(
                &Command::History(history::Command::Redo {}),
                &mut state,
                &mut ctx,
            )
            .unwrap();
        }
        assert_eq!(state.project, end, "{bpm} {rate}");
    }
}
