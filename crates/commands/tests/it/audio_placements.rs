//! #703: an audio placement stays inside its clip and never overlaps another one, whatever
//! command made it (`clip.audio.set_range`, `transport.set_tempo`, a restore), so that
//! deleting any placement can always be undone.

use crate::audio_clip::{import_at, range, span_of, with_whole};
use crate::clip_examples::{ClipFx, after, c, fx, with_audio_placement};
use ::project::{ClipContent, Frame, PlacementId, Span, Tick};
use commands::arrangement;
use commands::clip::{self, ClipTarget};
use commands::testing::{check_refusal, context};
use commands::{Command, State, Undo, run, transport};

fn insert_whole(state: &mut State, f: &ClipFx, start: i64, seed: u128) -> PlacementId {
    let (next, id) = with_whole(f, state.clone(), start, seed);
    *state = next;
    id
}

fn set_tempo(bpm: f64) -> Command {
    Command::Transport(transport::Command::SetTempo {
        bpm,
        placements: None,
    })
}

#[test]
fn a_range_change_that_would_leave_a_placement_past_the_clip_is_refused() {
    let f = fx();
    // The partial placement plays frames 11 025..33 075 of the clip (44 100 frames): a
    // shorter clip of 30 000 frames keeps its offset inside but its end outside.
    let (state, partial) = with_audio_placement(&f);
    let error = check_refusal(&range(&f, 0, 30_000), &state, &context()).unwrap();
    assert_eq!(error.code(), "invalid");
    assert!(error.to_string().contains(&partial.to_string()), "{error}");
    // 33 075 frames keep it exactly inside.
    run(&range(&f, 0, 33_075), &mut state.clone(), &mut context()).unwrap();
    // A looped clip keeps the strict rule (AU-6, #115, decides looped placements).
    let mut looped = state.clone();
    run(
        &c(clip::Command::AudioSetLoop {
            clip: f.audio_clip.clone(),
            looping: Some(::project::FrameLoop {
                start: Frame::new(0),
                end: Frame::new(1_000),
            }),
        }),
        &mut looped,
        &mut context(),
    )
    .unwrap();
    let error = check_refusal(&range(&f, 0, 30_000), &looped, &context()).unwrap();
    assert_eq!(error.code(), "invalid");
}

#[test]
fn a_growing_placement_stops_at_the_next_one_and_exact_lengths_are_checked() {
    let f = fx();
    // A whole placement at 0 (1 920 ticks) and another from 2 000.
    let (mut state, first) = with_whole(&f, f.state.clone(), 0, 11);
    let second = insert_whole(&mut state, &f, 2_000, 12);
    // Growing the clip to two seconds stops the first one before the second.
    let (grown, undo) = after(state.clone(), &range(&f, 0, 88_200));
    let Span::Frames { length, .. } = span_of(&grown, &f, &first) else {
        panic!()
    };
    assert!(length.get() < 88_200 && length.get() > 44_100, "{length:?}");
    let mut back = grown;
    run(&undo, &mut back, &mut context()).unwrap();
    assert_eq!(back.project, state.project);
    // Exact lengths (the internal field of an undo) are refused when they overlap the next
    // placement or end past the clip.
    let exact = |n: u64, frames: u64| {
        c(clip::Command::AudioSetRange {
            clip: f.audio_clip.clone(),
            start: Frame::new(0),
            frames: Frame::new(frames),
            placements: Some(vec![clip::PlacementFrames {
                placement: first.clone(),
                length: Frame::new(n),
            }]),
        })
    };
    assert!(check_refusal(&exact(45_100, 88_200), &state, &context()).is_err());
    let overlap = check_refusal(&exact(88_200, 88_200), &state, &context()).unwrap();
    assert_eq!(overlap.code(), "invalid");
    let message = overlap.to_string();
    assert!(
        message.contains(&first.to_string()) && message.contains(&second.to_string()),
        "{message}"
    );
    let past = check_refusal(&exact(44_101, 44_100), &state, &context()).unwrap();
    assert_eq!(past.code(), "invalid");
}

#[test]
fn a_faster_tempo_shortens_the_placement_that_would_run_into_the_next_and_undoes_exactly() {
    let f = fx();
    // One second is 1 920 ticks at 120 BPM and 3 840 at 240: the second placement starts
    // at 2 000, so the first one fits at 120 and would overlap it at 240.
    let (mut state, first) = with_whole(&f, f.state.clone(), 0, 21);
    let second = insert_whole(&mut state, &f, 2_000, 22);
    let (faster, undo) = after(state.clone(), &set_tempo(240.0));
    let Span::Frames { length, .. } = span_of(&faster, &f, &first) else {
        panic!()
    };
    assert!(length.get() < 44_100, "{length:?}");
    assert_eq!(span_of(&faster, &f, &second), span_of(&state, &f, &second));
    for p in faster.project.tracks[&f.audio_track].placements.values() {
        assert!(p.start.get() == 0 || p.start.get() == 2_000);
    }
    audio_placements_hold(&faster);
    // The undo gives the exact lengths back.
    let mut back = faster;
    run(&undo, &mut back, &mut context()).unwrap();
    assert_eq!(back.project, state.project);
    // Slower is fine, and undoing it comes back.
    let (slower, undo) = after(state.clone(), &set_tempo(60.0));
    let mut back = slower;
    run(&undo, &mut back, &mut context()).unwrap();
    assert_eq!(back.project, state.project);
    // An overlap the project already held does not block a tempo change.
    let mut held = state.clone();
    insert_whole(&mut held, &f, 100, 23);
    let lengths = |s: &State| -> Vec<Span> {
        s.project.tracks[&f.audio_track]
            .placements
            .values()
            .map(|p| p.span)
            .collect()
    };
    let before = lengths(&held);
    run(&set_tempo(121.0), &mut held, &mut context()).unwrap();
    assert_eq!(lengths(&held), before, "a held overlap is left as it is");
    // The internal exact lengths are checked: unknown, past the clip, zero.
    let exact = |id: &PlacementId, n: u64| {
        Command::Transport(transport::Command::SetTempo {
            bpm: 90.0,
            placements: Some(vec![clip::PlacementFrames {
                placement: id.clone(),
                length: Frame::new(n),
            }]),
        })
    };
    assert!(check_refusal(&exact(&first, 44_101), &state, &context()).is_ok());
    assert!(check_refusal(&exact(&first, 0), &state, &context()).is_ok());
    let unknown = PlacementId::generate(&mut ::project::IdGenerator::new(1, 99));
    assert!(check_refusal(&exact(&unknown, 10), &state, &context()).is_ok());
    run(&exact(&first, 40_000), &mut state.clone(), &mut context()).unwrap();
}

#[test]
fn a_restored_audio_clip_refuses_a_placement_ending_past_the_clip() {
    let f = fx();
    let (state, _) = with_whole(&f, f.state.clone(), 0, 31);
    let (deleted, undo) = after(
        state,
        &c(clip::Command::Delete {
            clip: f.audio_clip.clone(),
        }),
    );
    let Command::Clip(clip::Command::RestoreAudio {
        track,
        clip,
        snapshot,
        placements,
    }) = undo
    else {
        panic!("{undo:?}")
    };
    let restore = |length: u64, offset: u64| {
        let mut placements = placements.clone();
        placements[0].length = Frame::new(length);
        placements[0].offset = Frame::new(offset);
        c(clip::Command::RestoreAudio {
            track: track.clone(),
            clip: clip.clone(),
            snapshot: snapshot.clone(),
            placements,
        })
    };
    assert!(check_refusal(&restore(44_100, 0), &deleted, &context()).is_err());
    for (length, offset) in [(44_101, 0), (44_100, 1), (1, 44_100)] {
        let error = check_refusal(&restore(length, offset), &deleted, &context()).unwrap();
        assert_eq!(error.code(), "invalid", "{length} {offset}");
    }
    // A placement that overlaps one the track holds, or another restored with it.
    let mut held = deleted.clone();
    run(
        &import_at(&f, Some(ClipTarget::Arrangement { start: Tick(0) })),
        &mut held,
        &mut context(),
    )
    .unwrap();
    let with = |starts: &[i64], on: &State| {
        let mut placements = placements.clone();
        let mut next = placements[0].clone();
        placements.clear();
        for (i, start) in starts.iter().enumerate() {
            next.id = ::project::PlacementId::generate(&mut ::project::IdGenerator::new(
                1_700_000_000_000,
                40 + i as u128,
            ));
            next.start = Tick(*start);
            placements.push(next.clone());
        }
        let command = c(clip::Command::RestoreAudio {
            track: track.clone(),
            clip: clip.clone(),
            snapshot: snapshot.clone(),
            placements,
        });
        check_refusal(&command, on, &context())
    };
    // Two seconds' worth of ticks apart: fine; one tick apart: overlap.
    assert!(with(&[0, 3_840], &deleted).is_err());
    let error = with(&[0, 100], &deleted).unwrap();
    assert_eq!(error.code(), "invalid");
    // Against a placement the track already holds (the import covers ticks 0 to 4 000).
    assert!(with(&[5_000], &held).is_err());
    let error = with(&[100], &held).unwrap();
    assert_eq!(error.code(), "invalid");
}

/// A small deterministic generator: the sequences below are the same on every run.
struct Rng(u64);

impl Rng {
    fn next(&mut self, below: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % below
    }
}

fn audio_placements_hold(state: &State) {
    state.project.validate().unwrap();
    let tempo = state.project.tempo();
    for track in state.project.tracks.values() {
        let mut spans = Vec::new();
        for (id, p) in &track.placements {
            let Span::Frames { length, offset } = p.span else {
                continue;
            };
            let ClipContent::Audio { frames, media, .. } = &track.clips[&p.clip].content else {
                panic!("{id}")
            };
            assert!(
                offset.get() + length.get() <= frames.get(),
                "{id} past its clip"
            );
            let ticks = tempo
                .frames_to_ticks(length, media.sample_rate)
                .get()
                .max(1);
            spans.push((id.clone(), p.start.get(), p.start.get() + ticks));
        }
        for (i, (a, s, e)) in spans.iter().enumerate() {
            for (b, os, oe) in &spans[i + 1..] {
                assert!(!(os < e && s < oe), "{a} overlaps {b}");
            }
        }
    }
}

#[test]
fn after_any_sequence_of_commands_every_delete_can_be_undone() {
    let f = fx();
    for seed in 1..=40_u64 {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        // Odd seeds start from a placement that plays part of the clip (offset and length
        // short of the whole), even ones from no placement.
        let mut state = if seed % 2 == 1 {
            with_audio_placement(&f).0
        } else {
            f.state.clone()
        };
        let mut ctx = context();
        let mut undos: Vec<(Command, Command)> = Vec::new();
        for _ in 0..30 {
            let track = state.project.tracks[&f.audio_track].clone();
            let clips: Vec<_> = track.clips.keys().cloned().collect();
            let placements: Vec<_> = track.placements.keys().cloned().collect();
            let command = match rng.next(7) {
                0 => import_at(
                    &f,
                    Some(ClipTarget::Arrangement {
                        start: Tick(rng.next(30_000) as i64),
                    }),
                ),
                1 | 2 => {
                    let id = clips[rng.next(clips.len() as u64) as usize].clone();
                    let ClipContent::Audio { media, .. } = &track.clips[&id].content else {
                        continue;
                    };
                    let total = media.frames.get();
                    let start = rng.next(total / 2);
                    let frames = 1 + rng.next(total - start);
                    c(clip::Command::AudioSetRange {
                        clip: id,
                        start: Frame::new(start),
                        frames: Frame::new(frames),
                        placements: None,
                    })
                }
                3 => set_tempo(20.0 + rng.next(980) as f64),
                4 => Command::Arrangement(arrangement::Command::Place {
                    clip: clips[rng.next(clips.len() as u64) as usize].clone(),
                    start: Tick(rng.next(30_000) as i64),
                    length: None,
                    offset: None,
                }),
                5 if !placements.is_empty() => Command::Arrangement(arrangement::Command::Delete {
                    placements: vec![
                        placements[rng.next(placements.len() as u64) as usize].clone(),
                    ],
                }),
                6 => {
                    // An undo, then (half of the time) the command again: a redo.
                    if let Some((command, inverse)) = undos.pop() {
                        run(&inverse, &mut state, &mut ctx).expect("an undo never fails");
                        audio_placements_hold(&state);
                        if rng.next(2) == 0
                            && let Ok(out) = run(&command, &mut state, &mut ctx)
                        {
                            audio_placements_hold(&state);
                            if let Undo::Inverse(inverse) = out.undo {
                                undos.push((command, *inverse));
                            }
                        }
                    }
                    continue;
                }
                _ => continue,
            };
            let before = state.clone();
            let Ok(out) = run(&command, &mut state, &mut ctx) else {
                assert_eq!(state.project, before.project, "a refusal changes nothing");
                continue;
            };
            audio_placements_hold(&state);
            if let Undo::Inverse(inverse) = out.undo {
                undos.push((command, *inverse));
            }
            // Every delete of this state, then its undo, succeeds and restores it.
            let track = &state.project.tracks[&f.audio_track];
            let deletes: Vec<Command> = track
                .placements
                .keys()
                .map(|id| {
                    Command::Arrangement(arrangement::Command::Delete {
                        placements: vec![id.clone()],
                    })
                })
                .chain(
                    track
                        .clips
                        .keys()
                        .map(|id| c(clip::Command::Delete { clip: id.clone() })),
                )
                .collect();
            for delete in deletes {
                let mut probe = state.clone();
                let out = run(&delete, &mut probe, &mut ctx).unwrap();
                let Undo::Inverse(inverse) = out.undo else {
                    panic!("{delete:?}")
                };
                run(&inverse, &mut probe, &mut ctx)
                    .unwrap_or_else(|e| panic!("seed {seed}: {delete:?} then undo: {e:?}"));
                assert_eq!(probe.project, state.project, "seed {seed}");
            }
        }
    }
}
