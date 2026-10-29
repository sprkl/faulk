//! PN-2: `clip.notes.quantize`, `clip.notes.transpose` and `clip.notes.duplicate` beyond the
//! harness examples: edge cases (a note at the clip end, strength 0 and 1, merges), refusals that
//! leave the state intact, one history entry, exact undo and redo, and 100 000 notes in one command.

mod clip_examples;

use ::project::{ClipContent, ClipId, Note, Tick};
use clip_examples::{ClipFx, c, fx, n};
use commands::clip;
use commands::testing::{check, check_refusal, context};
use commands::{Command, CommandError, Output, State, Undo, run};
use std::collections::BTreeSet;

fn notes_of(state: &State, fx: &ClipFx, clip: &ClipId) -> BTreeSet<Note> {
    match &state.project.tracks[&fx.track].clips[clip].content {
        ClipContent::Midi { notes, .. } => notes.clone(),
        ClipContent::Audio { .. } => panic!("audio"),
    }
}

/// A clip of 3840 ticks holding exactly `notes`.
fn with_notes(notes: &[Note]) -> (ClipFx, State) {
    with_length_notes(3840, notes)
}

/// A clip of `length` ticks holding exactly `notes`.
fn with_length_notes(length: i64, notes: &[Note]) -> (ClipFx, State) {
    let f = fx();
    let mut state = f.state.clone();
    run(
        &c(clip::Command::SetContent {
            clip: f.clip.clone(),
            length: Tick(length),
            looping: None,
            notes: notes.to_vec(),
            placements: None,
        }),
        &mut state,
        &mut context(),
    )
    .unwrap();
    (f, state)
}

fn quantize(f: &ClipFx, notes: Option<Vec<Note>>, grid: i64, strength: u8) -> Command {
    c(clip::Command::NotesQuantize {
        clip: f.clip.clone(),
        notes,
        grid: Tick(grid),
        strength,
    })
}

fn refusal(state: &State, command: &Command) -> CommandError {
    check_refusal(command, state, &context()).unwrap()
}

/// Runs a command: the state after, the outcome.
fn go(state: &State, command: &Command) -> (State, commands::Outcome) {
    let mut after = state.clone();
    let out = run(command, &mut after, &mut context()).unwrap();
    (after, out)
}

fn undo(mut state: State, out: commands::Outcome) -> State {
    let Undo::Inverse(inverse) = out.undo else {
        panic!("no inverse")
    };
    inverse.apply(&mut state, &mut context()).unwrap();
    state
}

#[test]
fn quantize_moves_starts_to_the_nearest_line_and_keeps_lengths() {
    let (f, state) = with_notes(&[n(100, 60, 50, 100), n(130, 62, 40, 90), n(500, 64, 20, 80)]);
    let command = quantize(&f, None, 240, 100);
    check(&command, &state, &context()).unwrap();
    let (after, out) = go(&state, &command);
    assert_eq!(
        out.output,
        Output::Clip(clip::Output::NotesQuantize { quantized: 3 })
    );
    assert_eq!(
        notes_of(&after, &f, &f.clip),
        BTreeSet::from([n(0, 60, 50, 100), n(240, 62, 40, 90), n(480, 64, 20, 80)])
    );
    after.project.validate().unwrap();
    assert_eq!(after.history.undo.len(), state.history.undo.len() + 1);
    assert_eq!(undo(after, out).project, state.project);
}

#[test]
fn quantize_strength_zero_changes_nothing_and_records_nothing() {
    let (f, state) = with_notes(&[n(100, 60, 50, 100)]);
    let (after, out) = go(&state, &quantize(&f, None, 240, 0));
    assert_eq!(after, state);
    assert_eq!(
        out.output,
        Output::Clip(clip::Output::NotesQuantize { quantized: 0 })
    );
    assert_eq!((out.undo, out.events.len()), (Undo::None, 0));
}

#[test]
fn quantize_strength_one_moves_a_hundredth_and_fifty_half_of_the_way() {
    let (f, state) = with_notes(&[n(150, 60, 50, 100)]);
    for (strength, start) in [(1, 151), (50, 195), (99, 239), (100, 240)] {
        let (after, _) = go(&state, &quantize(&f, None, 240, strength));
        assert_eq!(
            notes_of(&after, &f, &f.clip),
            BTreeSet::from([n(start, 60, 50, 100)]),
            "strength {strength}"
        );
    }
    // A distance too short for the share to reach one tick: the note stays and nothing is recorded.
    let (_, state) = with_notes(&[n(10, 60, 50, 100)]);
    let (after, out) = go(&state, &quantize(&f, None, 240, 1));
    assert_eq!(after, state);
    assert_eq!(out.undo, Undo::None);
}

#[test]
fn quantize_a_note_whose_snap_would_cross_the_clip_end_stops_where_it_fits() {
    // Clip of 3840: the note of 480 ticks at 3000 would go to the line 3840, ending at 4320.
    let (f, state) = with_notes(&[n(3000, 60, 480, 100)]);
    let command = quantize(&f, None, 1920, 100);
    check(&command, &state, &context()).unwrap();
    let (after, out) = go(&state, &command);
    assert_eq!(
        out.output,
        Output::Clip(clip::Output::NotesQuantize { quantized: 1 })
    );
    assert_eq!(
        notes_of(&after, &f, &f.clip),
        BTreeSet::from([n(3360, 60, 480, 100)])
    );
    after.project.validate().unwrap();
    // A note that ends exactly on the clip end and is already as far as it can go does not move.
    let (f, state) = with_notes(&[n(3360, 60, 480, 100)]);
    let (after, out) = go(&state, &quantize(&f, None, 1920, 100));
    assert_eq!(after, state);
    assert_eq!(out.undo, Undo::None);
    // A note as long as the clip has no room: it goes to 0.
    let (f, state) = with_notes(&[n(0, 60, 3840, 100)]);
    let (after, out) = go(&state, &quantize(&f, None, 1920, 100));
    assert_eq!(after, state);
    assert_eq!(out.undo, Undo::None);
    // A note past the last line before the end, grid wider than the clip.
    let (f, state) = with_notes(&[n(3500, 60, 340, 100)]);
    let (after, _) = go(&state, &quantize(&f, None, 100_000, 100));
    assert_eq!(
        notes_of(&after, &f, &f.clip),
        BTreeSet::from([n(0, 60, 340, 100)])
    );
}

#[test]
fn quantize_rounds_halfway_to_the_later_line() {
    let (f, state) = with_notes(&[n(120, 60, 10, 100), n(119, 62, 10, 100)]);
    let (after, _) = go(&state, &quantize(&f, None, 240, 100));
    assert_eq!(
        notes_of(&after, &f, &f.clip),
        BTreeSet::from([n(240, 60, 10, 100), n(0, 62, 10, 100)])
    );
}

#[test]
fn quantize_only_the_chosen_notes_and_refuses_a_note_that_is_not_in_the_clip() {
    let (f, state) = with_notes(&[n(100, 60, 50, 100), n(130, 62, 40, 90)]);
    let (after, out) = go(
        &state,
        &quantize(&f, Some(vec![n(130, 62, 40, 90)]), 240, 100),
    );
    assert_eq!(
        notes_of(&after, &f, &f.clip),
        BTreeSet::from([n(100, 60, 50, 100), n(240, 62, 40, 90)])
    );
    assert_eq!(
        out.output,
        Output::Clip(clip::Output::NotesQuantize { quantized: 1 })
    );
    // An empty choice quantizes nothing (not "all").
    let (after, out) = go(&state, &quantize(&f, Some(vec![]), 240, 100));
    assert_eq!((after, out.undo), (state.clone(), Undo::None));
    assert_eq!(
        refusal(
            &state,
            &quantize(&f, Some(vec![n(1, 60, 50, 100)]), 240, 100)
        )
        .code(),
        "not_found"
    );
}

#[test]
fn quantize_refuses_a_bad_grid_a_bad_strength_and_a_bad_clip() {
    let (f, state) = with_notes(&[n(100, 60, 50, 100)]);
    for grid in [0, -240, (1_i64 << 40) + 1] {
        assert_eq!(
            refusal(&state, &quantize(&f, None, grid, 100)).code(),
            "invalid",
            "grid {grid}"
        );
    }
    assert_eq!(
        refusal(&state, &quantize(&f, None, 240, 101)).code(),
        "invalid"
    );
    let audio = c(clip::Command::NotesQuantize {
        clip: f.audio_clip.clone(),
        notes: None,
        grid: Tick(240),
        strength: 100,
    });
    assert_eq!(refusal(&state, &audio).code(), "invalid");
    let missing = c(clip::Command::NotesQuantize {
        clip: ClipId::generate(&mut ::project::IdGenerator::new(1, 2)),
        notes: None,
        grid: Tick(240),
        strength: 100,
    });
    assert_eq!(refusal(&state, &missing).code(), "not_found");
    // An empty clip: nothing to do.
    let (f, state) = with_notes(&[]);
    let (after, out) = go(&state, &quantize(&f, None, 240, 100));
    assert_eq!((after, out.undo), (state, Undo::None));
}

#[test]
fn a_note_cannot_have_a_negative_position_so_quantize_never_meets_one() {
    let (f, state) = with_notes(&[n(0, 60, 50, 100)]);
    let negative = c(clip::Command::NotesAdd {
        clip: f.clip.clone(),
        notes: vec![n(-10, 60, 50, 100)],
    });
    assert_eq!(refusal(&state, &negative).code(), "invalid");
    let listed = quantize(&f, Some(vec![n(-10, 60, 50, 100)]), 240, 100);
    assert_eq!(refusal(&state, &listed).code(), "not_found");
}

#[test]
fn quantize_that_makes_notes_equal_merges_them_and_undo_gives_every_note_back() {
    // Two notes of the same pitch, length and velocity, 10 ticks apart: one after quantizing.
    let (f, state) = with_notes(&[n(100, 60, 50, 100), n(110, 60, 50, 100), n(960, 64, 50, 90)]);
    let command = quantize(&f, None, 240, 100);
    check(&command, &state, &context()).unwrap();
    let (after, out) = go(&state, &command);
    assert_eq!(
        notes_of(&after, &f, &f.clip),
        BTreeSet::from([n(0, 60, 50, 100), n(960, 64, 50, 90)])
    );
    let Undo::Inverse(inverse) = &out.undo else {
        panic!()
    };
    assert_eq!(inverse.name(), "clip.set_content");
    let back = undo(after, out);
    assert_eq!(back.project, state.project);
    // A chosen note landing on a note that stays merges too.
    let (f, state) = with_notes(&[n(0, 60, 50, 100), n(10, 60, 50, 100)]);
    let command = quantize(&f, Some(vec![n(10, 60, 50, 100)]), 240, 100);
    check(&command, &state, &context()).unwrap();
    let (after, out) = go(&state, &command);
    assert_eq!(
        notes_of(&after, &f, &f.clip),
        BTreeSet::from([n(0, 60, 50, 100)])
    );
    assert_eq!(undo(after, out).project, state.project);
}

#[test]
fn quantize_without_a_merge_undoes_with_the_opposite_moves() {
    let (f, state) = with_notes(&[n(100, 60, 50, 100), n(500, 62, 50, 100)]);
    let (after, out) = go(&state, &quantize(&f, None, 240, 100));
    let Undo::Inverse(inverse) = &out.undo else {
        panic!()
    };
    assert_eq!(inverse.name(), "clip.notes.update");
    assert_eq!(undo(after, out).project, state.project);
}

#[test]
fn transpose_moves_pitches_and_keeps_everything_else() {
    let (f, state) = with_notes(&[
        Note {
            cents: 7,
            chance: 50,
            repeat: 3,
            ..n(0, 60, 480, 100)
        },
        n(960, 127, 480, 90),
    ]);
    let chosen = vec![Note {
        cents: 7,
        chance: 50,
        repeat: 3,
        ..n(0, 60, 480, 100)
    }];
    let transpose = |notes: Vec<Note>, semitones| {
        c(clip::Command::NotesTranspose {
            clip: f.clip.clone(),
            notes,
            semitones,
        })
    };
    let command = transpose(chosen.clone(), 12);
    check(&command, &state, &context()).unwrap();
    let (after, out) = go(&state, &command);
    assert_eq!(
        out.output,
        Output::Clip(clip::Output::NotesTranspose { transposed: 1 })
    );
    assert!(notes_of(&after, &f, &f.clip).contains(&Note {
        pitch: 72,
        ..chosen[0]
    }));
    assert_eq!(undo(after, out).project, state.project);
    // 0 semitones and an empty choice change nothing.
    for command in [transpose(chosen.clone(), 0), transpose(vec![], 5)] {
        let (after, out) = go(&state, &command);
        assert_eq!((after, out.undo), (state.clone(), Undo::None));
    }
    // Past either end of the keyboard: the whole batch is refused, never clamped.
    let both = vec![chosen[0], n(960, 127, 480, 90)];
    assert_eq!(
        refusal(&state, &transpose(both.clone(), 1)).code(),
        "invalid"
    );
    assert_eq!(
        refusal(&state, &transpose(chosen.clone(), -61)).code(),
        "invalid"
    );
    assert_eq!(
        refusal(&state, &transpose(chosen.clone(), i32::MAX)).code(),
        "invalid"
    );
    assert_eq!(
        refusal(&state, &transpose(chosen, i32::MIN)).code(),
        "invalid"
    );
    assert_eq!(
        refusal(&state, &transpose(vec![n(1, 60, 1, 1)], 1)).code(),
        "not_found"
    );
}

#[test]
fn transpose_that_lands_on_another_note_merges_and_undoes() {
    let (f, state) = with_notes(&[n(0, 60, 480, 100), n(0, 62, 480, 100)]);
    let command = c(clip::Command::NotesTranspose {
        clip: f.clip.clone(),
        notes: vec![n(0, 60, 480, 100)],
        semitones: 2,
    });
    check(&command, &state, &context()).unwrap();
    let (after, out) = go(&state, &command);
    assert_eq!(
        notes_of(&after, &f, &f.clip),
        BTreeSet::from([n(0, 62, 480, 100)])
    );
    assert_eq!(undo(after, out).project, state.project);
}

#[test]
fn duplicate_follows_the_selection_by_default_and_takes_an_offset() {
    let (f, state) = with_length_notes(3 * 3840, &[n(0, 60, 480, 100), n(480, 62, 480, 100)]);
    let duplicate = |notes: Vec<Note>, offset: Option<i64>| {
        c(clip::Command::NotesDuplicate {
            clip: f.clip.clone(),
            notes,
            offset: offset.map(Tick),
        })
    };
    let all = vec![n(0, 60, 480, 100), n(480, 62, 480, 100)];
    let command = duplicate(all.clone(), None);
    check(&command, &state, &context()).unwrap();
    let (after, out) = go(&state, &command);
    assert_eq!(
        out.output,
        Output::Clip(clip::Output::NotesDuplicate { added: 2 })
    );
    assert_eq!(
        notes_of(&after, &f, &f.clip),
        BTreeSet::from([
            n(0, 60, 480, 100),
            n(480, 62, 480, 100),
            n(3840, 60, 480, 100),
            n(4320, 62, 480, 100)
        ])
    );
    assert_eq!(after.history.undo.len(), state.history.undo.len() + 1);
    assert_eq!(undo(after, out).project, state.project);
    // A negative offset copies earlier; the last end may touch the clip end.
    let (after, _) = go(&state, &duplicate(vec![all[1]], Some(3 * 3840 - 960)));
    assert!(notes_of(&after, &f, &f.clip).contains(&n(3 * 3840 - 480, 62, 480, 100)));
    let (after, _) = go(&state, &duplicate(vec![all[1]], Some(-480)));
    assert!(notes_of(&after, &f, &f.clip).contains(&n(0, 62, 480, 100)));
    // Copies already there change nothing; only the new ones count and are undone.
    let (f3, state3) = with_notes(&[
        n(0, 60, 480, 100),
        n(480, 60, 480, 100),
        n(960, 62, 480, 100),
    ]);
    let dup3 = |notes: Vec<Note>| {
        c(clip::Command::NotesDuplicate {
            clip: f3.clip.clone(),
            notes,
            offset: Some(Tick(480)),
        })
    };
    let (after, out) = go(
        &state3,
        &dup3(vec![n(0, 60, 480, 100), n(480, 60, 480, 100)]),
    );
    assert_eq!(
        out.output,
        Output::Clip(clip::Output::NotesDuplicate { added: 1 })
    );
    assert_eq!(undo(after, out).project, state3.project);
    let (after, out) = go(&state3, &dup3(vec![n(0, 60, 480, 100)]));
    assert_eq!((after, out.undo), (state3.clone(), Undo::None));
    // Refusals: past either end of the clip, an offset of 0, a note that is not there.
    assert_eq!(
        refusal(&state, &duplicate(all.clone(), Some(3 * 3840 - 960 + 1))).code(),
        "invalid"
    );
    assert_eq!(
        refusal(&state, &duplicate(all.clone(), Some(-1))).code(),
        "invalid"
    );
    assert_eq!(
        refusal(&state, &duplicate(all.clone(), Some(0))).code(),
        "invalid"
    );
    assert_eq!(
        refusal(&state, &duplicate(vec![n(5, 60, 5, 5)], None)).code(),
        "not_found"
    );
    // The default offset is whole bars from the bar of the first note: a selection of one bar or
    // less goes to the next bar, a selection across a bar line takes both bars.
    let (fb, bars) = with_length_notes(4 * 3840, &[n(3840 + 100, 60, 480, 100)]);
    let dup = |notes: Vec<Note>| {
        c(clip::Command::NotesDuplicate {
            clip: fb.clip.clone(),
            notes,
            offset: None,
        })
    };
    let (after, _) = go(&bars, &dup(vec![n(3840 + 100, 60, 480, 100)]));
    assert!(notes_of(&after, &fb, &fb.clip).contains(&n(2 * 3840 + 100, 60, 480, 100)));
    let across = [
        n(3840 + 3800, 60, 480, 100),
        n(2 * 3840 + 100, 62, 100, 100),
    ];
    let (fa, spanning) = with_length_notes(6 * 3840, &across);
    let (after, _) = go(
        &spanning,
        &c(clip::Command::NotesDuplicate {
            clip: fa.clip.clone(),
            notes: across.to_vec(),
            offset: None,
        }),
    );
    assert!(notes_of(&after, &fa, &fa.clip).contains(&n(3 * 3840 + 3800, 60, 480, 100)));
    assert!(notes_of(&after, &fa, &fa.clip).contains(&n(4 * 3840 + 100, 62, 100, 100)));
    // The whole clip as the selection has no room to follow itself.
    let (f2, state2) = with_notes(&[n(0, 60, 3840, 100)]);
    let whole = c(clip::Command::NotesDuplicate {
        clip: f2.clip.clone(),
        notes: vec![n(0, 60, 3840, 100)],
        offset: None,
    });
    assert_eq!(refusal(&state2, &whole).code(), "invalid");
    // An empty choice does nothing.
    let (after, out) = go(&state, &duplicate(vec![], None));
    assert_eq!((after, out.undo), (state, Undo::None));
}

#[test]
fn a_hundred_thousand_notes_quantize_transpose_and_duplicate_in_one_command_each() {
    let (f, mut state) = with_notes(&[]);
    let mut ctx = context();
    run(
        &c(clip::Command::SetLength {
            placements: None,
            clip: f.clip.clone(),
            length: Tick(2_000_000),
            trim: false,
        }),
        &mut state,
        &mut ctx,
    )
    .unwrap();
    // Distinct starts 10 ticks apart, so a coarse grid makes many of them equal: a merge at scale.
    let batch: Vec<Note> = (0..100_000).map(|i| n(i * 10, 60, 5, 100)).collect();
    run(
        &c(clip::Command::NotesAdd {
            clip: f.clip.clone(),
            notes: batch.clone(),
        }),
        &mut state,
        &mut ctx,
    )
    .unwrap();
    let before = state.clone();
    let entries = state.history.undo.len();
    let out = run(&quantize(&f, None, 960, 100), &mut state, &mut ctx).unwrap();
    assert_eq!(state.history.undo.len(), entries + 1);
    assert_eq!(notes_of(&state, &f, &f.clip).len(), 1043);
    let state = undo(state, out);
    assert_eq!(state.project, before.project);
    let mut state = before.clone();
    let out = run(
        &c(clip::Command::NotesTranspose {
            clip: f.clip.clone(),
            notes: batch.clone(),
            semitones: 7,
        }),
        &mut state,
        &mut ctx,
    )
    .unwrap();
    assert_eq!(notes_of(&state, &f, &f.clip).len(), 100_000);
    assert_eq!(undo(state, out).project, before.project);
    let mut state = before.clone();
    run(
        &c(clip::Command::NotesDuplicate {
            clip: f.clip.clone(),
            notes: batch,
            offset: Some(Tick(1_000_000)),
        }),
        &mut state,
        &mut ctx,
    )
    .unwrap();
    assert_eq!(notes_of(&state, &f, &f.clip).len(), 200_000);
    state.project.validate().unwrap();
}
