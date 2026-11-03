//! #500: the replace mode and the quantization on input of `record.commit_take`, and their
//! settings in `record.set_settings` (`docs/specs/03-enregistrement-midi.md` 4.2 and 6).

mod record_examples;

use ::project::{ClipContent, InputGrid, Note, Tick};
use commands::engine_msgs::Synced;
use commands::record::{Command as R, Output as RO, RecordTarget, TakeNote, TakeTrack};
use commands::testing::context;
use commands::{Command, CommandError, Output, ProjectEvent, State, history};
use record_examples::{Rec, note, rec, take};

fn synced(r: &Rec) -> Synced {
    let mut s = Synced::new(r.state.clone(), context(), 1);
    s.setup().unwrap();
    s
}

fn run(s: &mut Synced, c: R) -> (Output, Vec<ProjectEvent>) {
    let (outcome, _) = s
        .run(&Command::Record(c.clone()))
        .unwrap_or_else(|e| panic!("{c:?}: {e:?}"));
    (outcome.output, outcome.events)
}

fn notes_of(state: &State, clip: &::project::ClipId) -> Vec<Note> {
    let clip = state
        .project
        .tracks
        .values()
        .find_map(|t| t.clips.get(clip))
        .unwrap();
    match &clip.content {
        ClipContent::Midi { notes, .. } => notes.iter().copied().collect(),
        ClipContent::Audio { .. } => panic!("audio"),
    }
}

/// A copy of the fixture with `notes` added to `clip`.
fn with_notes(r: &Rec, clip: &::project::ClipId, notes: Vec<Note>) -> Rec {
    let mut r = r.clone();
    commands::run(
        &Command::Clip(commands::clip::Command::NotesAdd {
            clip: clip.clone(),
            notes,
        }),
        &mut r.state,
        &mut context(),
    )
    .unwrap();
    r
}

fn commit_with(
    r: &Rec,
    target: RecordTarget,
    tracks: Vec<TakeTrack>,
    replace: Option<bool>,
    grid: Option<InputGrid>,
    strength: Option<u8>,
) -> (State, RO) {
    let mut state = r.state.clone();
    let outcome = commands::run(
        &Command::Record(R::CommitTake {
            take: None,
            target,
            shift_ms: 0.0,
            tracks,
            replace,
            quantize_on_input: grid,
            quantize_strength: strength,
        }),
        &mut state,
        &mut context(),
    )
    .unwrap();
    state.project.validate().unwrap();
    match outcome.output {
        Output::Record(o) => (state, o),
        o => panic!("{o:?}"),
    }
}

fn scene(r: &Rec) -> RecordTarget {
    RecordTarget::Scene {
        scene: r.scene.clone(),
    }
}

// The grid clip loops over one bar (3840 ticks) with notes at 0 and 1920 (length 480,
// velocity 90); the take's anchor 3840 is its position 0.

#[test]
fn replace_removes_the_notes_that_start_in_the_recorded_range_and_keeps_the_rest() {
    let r = rec();
    // One note at position 1900 for 100 ticks: the range is [1900, 2000).
    let tracks = take(&r, vec![note(3840 + 1900, 61, 100)], Some(3840));
    let (state, out) = commit_with(&r, scene(&r), tracks.clone(), Some(true), None, None);
    assert!(matches!(out, RO::CommitTake { replaced: 1, .. }));
    assert_eq!(
        notes_of(&state, &r.grid_clip),
        vec![
            Note::new(Tick(0), 48, Tick(480), 90),
            Note::new(Tick(1900), 61, Tick(100), 100),
        ]
    );
    // Overdub (the default) keeps everything.
    let (state, out) = commit_with(&r, scene(&r), tracks, None, None, None);
    assert!(matches!(out, RO::CommitTake { replaced: 0, .. }));
    assert_eq!(notes_of(&state, &r.grid_clip).len(), 3);
}

#[test]
fn replace_follows_the_loop_round_and_covers_a_long_take_whole() {
    let r = rec();
    // Position 3800 to the end of the loop, then 200 ticks into the next lap: the note at
    // 0 goes, the one at 1920 stays.
    let tracks = take(
        &r,
        vec![note(3840 + 3800, 61, 100), note(3840 * 2 + 100, 62, 100)],
        Some(3840),
    );
    let (state, out) = commit_with(&r, scene(&r), tracks, Some(true), None, None);
    assert!(matches!(out, RO::CommitTake { replaced: 1, .. }));
    assert!(
        notes_of(&state, &r.grid_clip).contains(&Note::new(Tick(1920), 50, Tick(480), 90)),
        "outside the range"
    );
    // A take longer than the loop replaces the whole clip.
    let tracks = take(
        &r,
        vec![note(3840 + 10, 61, 100), note(3840 * 3 + 10, 62, 100)],
        Some(3840),
    );
    let (state, out) = commit_with(&r, scene(&r), tracks, Some(true), None, None);
    assert!(matches!(out, RO::CommitTake { replaced: 2, .. }));
    assert_eq!(notes_of(&state, &r.grid_clip).len(), 2);
}

#[test]
fn replace_in_the_arrangement_stays_inside_the_placement_it_overlaps() {
    let r = rec();
    let r = with_notes(
        &r,
        &r.placed_clip,
        vec![
            Note::new(Tick(100), 40, Tick(100), 90),
            Note::new(Tick(3000), 41, Tick(100), 90),
        ],
    );
    // The placement starts at 7680: the take plays 7730 to 7930, positions 50 to 250.
    let tracks = take(&r, vec![note(7730, 60, 200)], None);
    let (state, out) = commit_with(
        &r,
        RecordTarget::Arrangement {},
        tracks,
        Some(true),
        None,
        None,
    );
    assert!(matches!(out, RO::CommitTake { replaced: 1, .. }));
    assert_eq!(
        notes_of(&state, &r.placed_clip),
        vec![
            Note::new(Tick(50), 60, Tick(200), 100),
            Note::new(Tick(3000), 41, Tick(100), 90),
        ]
    );
}

#[test]
fn replace_with_the_same_notes_writes_nothing() {
    let r = rec();
    // The take plays exactly the clip's note at 1920: removed and put back, no change.
    let same = vec![TakeNote {
        start: Tick(3840 + 1920),
        length: Tick(480),
        pitch: 50,
        velocity: 90,
    }];
    let (state, out) = commit_with(
        &r,
        scene(&r),
        take(&r, same, Some(3840)),
        Some(true),
        None,
        None,
    );
    assert!(matches!(out, RO::CommitTake { replaced: 0, ref clips, .. } if clips.is_empty()));
    assert_eq!(state.project, r.state.project);
}

#[test]
fn replace_is_one_history_entry_and_one_undo_gives_the_notes_back() {
    let r = rec();
    let mut s = synced(&r);
    run(
        &mut s,
        R::SetSettings {
            offset_ms: None,
            replace: Some(true),
            quantize_on_input: None,
            quantize_strength: None,
        },
    );
    run(
        &mut s,
        R::Start {
            target: Some(scene(&r)),
            latency_ms: None,
        },
    );
    run(&mut s, R::Stop {});
    let before = s.state.project.clone();
    let entries = s.state.history.undo.len();
    let (out, _) = run(
        &mut s,
        R::CommitTake {
            take: Some(1),
            target: scene(&r),
            shift_ms: 0.0,
            tracks: take(&r, vec![note(3840 + 1900, 61, 100)], Some(3840)),
            replace: None,
            quantize_on_input: None,
            quantize_strength: None,
        },
    );
    assert!(matches!(
        out,
        Output::Record(RO::CommitTake { replaced: 1, .. })
    ));
    assert_eq!(s.state.history.undo.len(), entries + 1);
    let after = s.state.project.clone();
    s.run(&Command::History(history::Command::Undo {})).unwrap();
    assert_eq!(s.state.project, before, "the replaced note is back");
    s.run(&Command::History(history::Command::Redo {})).unwrap();
    assert_eq!(s.state.project, after);
}

#[test]
fn quantize_on_input_moves_only_the_notes_of_the_take() {
    let r = rec();
    let r = with_notes(
        &r,
        &r.grid_clip,
        vec![Note::new(Tick(77), 30, Tick(10), 90)],
    );
    let tracks = take(
        &r,
        vec![
            note(3840 + 100, 60, 50),
            note(3840 + 130, 62, 50),
            note(3840 + 3700, 64, 100),
        ],
        Some(3840),
    );
    // Sixteenth, full strength: 100 -> 0, 130 -> 240, 3700 -> 3600. The clip's own off-grid
    // note stays.
    let (state, _) = commit_with(
        &r,
        scene(&r),
        tracks.clone(),
        None,
        Some(InputGrid::Sixteenth),
        Some(100),
    );
    let notes = notes_of(&state, &r.grid_clip);
    for want in [
        Note::new(Tick(0), 60, Tick(50), 100),
        Note::new(Tick(240), 62, Tick(50), 100),
        Note::new(Tick(3600), 64, Tick(100), 100),
        Note::new(Tick(77), 30, Tick(10), 90),
    ] {
        assert!(notes.contains(&want), "{want:?} in {notes:?}");
    }
    // A quarter note grid would take 3700 to 3840, the clip's end: the note stops where it
    // still fits (3840 - 100).
    let (state, _) = commit_with(
        &r,
        scene(&r),
        tracks.clone(),
        None,
        Some(InputGrid::Quarter),
        Some(100),
    );
    assert!(notes_of(&state, &r.grid_clip).contains(&Note::new(Tick(3740), 64, Tick(100), 100)));
    // Half strength: 130 moves half of the way to 240.
    let (state, _) = commit_with(
        &r,
        scene(&r),
        tracks.clone(),
        None,
        Some(InputGrid::Sixteenth),
        Some(50),
    );
    assert!(notes_of(&state, &r.grid_clip).contains(&Note::new(Tick(185), 62, Tick(50), 100)));
    // Strength 0, and no grid, leave the notes as played.
    let (state, _) = commit_with(
        &r,
        scene(&r),
        tracks.clone(),
        None,
        Some(InputGrid::Sixteenth),
        Some(0),
    );
    assert!(notes_of(&state, &r.grid_clip).contains(&Note::new(Tick(130), 62, Tick(50), 100)));
    let (state, _) = commit_with(&r, scene(&r), tracks, None, None, None);
    assert!(notes_of(&state, &r.grid_clip).contains(&Note::new(Tick(100), 60, Tick(50), 100)));
}

#[test]
fn the_session_settings_apply_to_a_commit_and_the_input_overrides_them() {
    let mut r = rec();
    r.state.session.record.quantize_on_input = InputGrid::Quarter;
    r.state.session.record.quantize_strength = 100;
    let tracks = take(&r, vec![note(3840 + 500, 60, 50)], Some(3840));
    let (state, _) = commit_with(&r, scene(&r), tracks.clone(), None, None, None);
    assert!(notes_of(&state, &r.grid_clip).contains(&Note::new(Tick(960), 60, Tick(50), 100)));
    let (state, _) = commit_with(
        &r,
        scene(&r),
        tracks.clone(),
        None,
        Some(InputGrid::None),
        None,
    );
    assert!(notes_of(&state, &r.grid_clip).contains(&Note::new(Tick(500), 60, Tick(50), 100)));
    let mut state = r.state.clone();
    let result = commands::run(
        &Command::Record(R::CommitTake {
            take: None,
            target: scene(&r),
            shift_ms: 0.0,
            tracks,
            replace: None,
            quantize_on_input: None,
            quantize_strength: Some(101),
        }),
        &mut state,
        &mut context(),
    );
    assert!(matches!(result, Err(CommandError::Invalid { .. })));
}

#[test]
fn an_empty_cell_takes_quantized_notes_and_has_nothing_to_replace() {
    let r = rec();
    let tracks = take(&r, vec![note(3840 + 130, 60, 50)], None);
    let target = RecordTarget::Scene {
        scene: r.empty.clone(),
    };
    let (state, out) = commit_with(
        &r,
        target,
        tracks,
        Some(true),
        Some(InputGrid::Eighth),
        Some(100),
    );
    let RO::CommitTake {
        clips, replaced, ..
    } = out
    else {
        panic!()
    };
    assert_eq!(replaced, 0);
    assert_eq!(
        notes_of(&state, &clips[0].clip),
        vec![Note::new(Tick(0), 60, Tick(50), 100)]
    );
}

#[test]
fn the_settings_change_one_field_at_a_time_and_are_checked() {
    let r = rec();
    let mut s = synced(&r);
    let set = |replace, grid, strength| R::SetSettings {
        offset_ms: None,
        replace,
        quantize_on_input: grid,
        quantize_strength: strength,
    };
    let (out, events) = run(
        &mut s,
        set(Some(true), Some(InputGrid::EighthTriplet), Some(60)),
    );
    assert_eq!(
        out,
        Output::Record(RO::SetSettings {
            previous_offset_ms: 0,
            previous_replace: false,
            previous_quantize_on_input: InputGrid::None,
            previous_quantize_strength: 100,
        })
    );
    assert_eq!(
        events,
        vec![ProjectEvent::RecordSettingsChanged {
            offset_ms: 0,
            replace: true,
            quantize_on_input: InputGrid::EighthTriplet,
            quantize_strength: 60,
        }]
    );
    // A field left out stays; nothing at all changes nothing and says nothing.
    let (_, events) = run(&mut s, set(Some(false), None, None));
    assert_eq!(events.len(), 1);
    assert_eq!(s.state.session.record.quantize_strength, 60);
    assert_eq!(
        s.state.session.record.quantize_on_input,
        InputGrid::EighthTriplet
    );
    let (_, events) = run(&mut s, set(None, None, None));
    assert!(events.is_empty());
    assert!(s.run(&Command::Record(set(None, None, Some(101)))).is_err());
    assert_eq!(s.state.session.record.quantize_strength, 60);
    assert_eq!(
        s.state.history.undo.len(),
        r.state.history.undo.len(),
        "no history entry"
    );
}

#[test]
fn the_input_grids_are_pinned_to_their_ids_and_ticks() {
    // The ids and steps of the piano roll's quantize field (`QUANTIZE_GRIDS` in
    // `web/src/views/piano-roll/roll.ts`): a change on either side fails here.
    let pinned = [
        ("none", InputGrid::None, None),
        ("quarter", InputGrid::Quarter, Some(960)),
        ("eighth", InputGrid::Eighth, Some(480)),
        ("sixteenth", InputGrid::Sixteenth, Some(240)),
        ("thirty-second", InputGrid::ThirtySecond, Some(120)),
        ("eighth-triplet", InputGrid::EighthTriplet, Some(320)),
        ("sixteenth-triplet", InputGrid::SixteenthTriplet, Some(160)),
    ];
    for (id, grid, ticks) in pinned {
        assert_eq!(serde_json::to_value(grid).unwrap(), serde_json::json!(id));
        assert_eq!(grid.ticks(), ticks);
    }
}
