//! RE-2 to RE-5: the `record.*` commands. A take starts and ends with its engine messages,
//! a stop or a pause of the transport ends it too, and `record.commit_take` writes it as one
//! history entry that one undo removes, on the three targets (a grid clip, an empty cell,
//! the arrangement).

use crate::record_examples;
use ::project::{ClipContent, Note, Tick, TickLoop};
use commands::engine_msgs::Synced;
use commands::record::{Command as R, Output as RO, RecordTarget, TakeClip};
use commands::testing::{check, context};
use commands::{
    Command, CommandError, ControlMsg, Output, ProjectEvent, State, TakeState, history, transport,
};
use engine::msg::{SceneSlot, TrackSlot};
use record_examples::{Rec, examples, note, rec, take};

fn synced(r: &Rec) -> Synced {
    let mut s = Synced::new(r.state.clone(), context(), 1);
    s.setup().unwrap();
    s
}

fn run(s: &mut Synced, c: R) -> (Output, Vec<ProjectEvent>, Vec<ControlMsg>) {
    let (outcome, _) = s
        .run(&Command::Record(c.clone()))
        .unwrap_or_else(|e| panic!("{c:?}: {e:?}"));
    (outcome.output, outcome.events, outcome.control)
}

fn refused(s: &mut Synced, c: R) -> CommandError {
    match s.run(&Command::Record(c)) {
        Err(commands::engine_msgs::SyncError::Refused(e)) => e,
        other => panic!("{other:?}"),
    }
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

#[test]
fn every_example_meets_the_contract_and_validates() {
    for (command, state) in examples() {
        check(&command, &state, &context()).unwrap();
        let mut s = state.clone();
        commands::run(&command, &mut s, &mut context()).unwrap();
        s.project.validate().unwrap();
    }
}

#[test]
fn a_take_starts_launches_the_scene_records_and_counts_in_from_a_stopped_transport() {
    let r = rec();
    let mut s = synced(&r);
    let (out, events, control) = run(
        &mut s,
        R::Start {
            target: Some(RecordTarget::Scene {
                scene: r.scene.clone(),
            }),
            latency_ms: Some(20.0),
        },
    );
    let RO::Start {
        take,
        tracks,
        shift_ms,
        count_in,
        ..
    } = (match out {
        Output::Record(o) => o,
        o => panic!("{o:?}"),
    })
    else {
        panic!()
    };
    assert_eq!(take, 1);
    assert_eq!(tracks, vec![r.track.clone()]);
    assert_eq!(shift_ms, 20.0);
    assert!(
        count_in,
        "one bar of count-in by default, the transport stopped"
    );
    assert_eq!(
        events,
        vec![ProjectEvent::RecordingChanged {
            take: 1,
            state: TakeState::Recording
        }]
    );
    // The scene's clip first, then the capture, then the play.
    assert_eq!(
        control,
        vec![
            ControlMsg::LaunchClip {
                track: TrackSlot(0),
                scene: SceneSlot(0),
            },
            ControlMsg::SetRecording {
                recording: true,
                shift_us: 20_000
            },
            ControlMsg::Play { from: Tick(0) },
        ]
    );
    // A second start while the take runs or waits is refused.
    assert!(matches!(
        refused(
            &mut s,
            R::Start {
                target: None,
                latency_ms: None
            }
        ),
        CommandError::Invalid { .. }
    ));
    // The stop ends the capture and keeps the transport running.
    let (out, events, control) = run(&mut s, R::Stop {});
    assert_eq!(out, Output::Record(RO::Stop { take: Some(1) }));
    assert_eq!(
        events,
        vec![ProjectEvent::RecordingChanged {
            take: 1,
            state: TakeState::Stopped
        }]
    );
    assert_eq!(
        control,
        vec![ControlMsg::SetRecording {
            recording: false,
            shift_us: 0
        }]
    );
    assert!(s.ctx.engine.playing);
    // Still waiting to be written: refused again; a discard frees it.
    assert!(matches!(
        refused(
            &mut s,
            R::Start {
                target: None,
                latency_ms: None
            }
        ),
        CommandError::Invalid { .. }
    ));
    let (out, events, control) = run(&mut s, R::DiscardTake {});
    assert_eq!(out, Output::Record(RO::DiscardTake { take: Some(1) }));
    assert_eq!(
        events,
        vec![ProjectEvent::RecordingChanged {
            take: 1,
            state: TakeState::Discarded
        }]
    );
    assert_eq!(
        control,
        vec![ControlMsg::SetRecording {
            recording: false,
            shift_us: 0
        }]
    );
    // On a running transport: no launch, no count-in, no second play.
    let (out, _, control) = run(
        &mut s,
        R::Start {
            target: Some(RecordTarget::Scene {
                scene: r.scene.clone(),
            }),
            latency_ms: None,
        },
    );
    assert!(matches!(
        out,
        Output::Record(RO::Start {
            take: 2,
            count_in: false,
            ..
        })
    ));
    assert_eq!(
        control,
        vec![ControlMsg::SetRecording {
            recording: true,
            shift_us: 0
        }]
    );
}

#[test]
fn a_transport_stop_or_pause_ends_the_capture() {
    for end in [
        transport::Command::Stop {},
        transport::Command::Pause { tick: Tick(960) },
    ] {
        let r = rec();
        let mut s = synced(&r);
        run(
            &mut s,
            R::Start {
                target: None,
                latency_ms: None,
            },
        );
        let (outcome, _) = s.run(&Command::Transport(end.clone())).unwrap();
        assert!(
            outcome.events.contains(&ProjectEvent::RecordingChanged {
                take: 1,
                state: TakeState::Stopped
            }),
            "{end:?}"
        );
        assert!(outcome.control.contains(&ControlMsg::SetRecording {
            recording: false,
            shift_us: 0
        }));
        // Once: a second stop does not end it again.
        let (outcome, _) = s.run(&Command::Transport(end)).unwrap();
        assert!(outcome.events.is_empty());
    }
}

#[test]
fn start_is_refused_without_an_armed_midi_track_or_with_a_bad_latency() {
    let r = rec();
    let mut state = r.state.clone();
    state.session.armed.clear();
    let mut s = Synced::new(state, context(), 1);
    s.setup().unwrap();
    assert!(matches!(
        refused(&mut s, R::Start {
            target: None,
            latency_ms: None
        }),
        CommandError::Invalid { field, .. } if field == "tracks"
    ));
    let mut s = synced(&r);
    for latency in [-1.0, f64::NAN, 1001.0] {
        assert!(matches!(
            refused(&mut s, R::Start {
                target: None,
                latency_ms: Some(latency)
            }),
            CommandError::Invalid { field, .. } if field == "latency_ms"
        ));
    }
    assert!(s.ctx.engine.take.is_none(), "a refusal starts nothing");
}

#[test]
fn the_offset_is_session_state_in_range_and_shifts_the_take() {
    let r = rec();
    let mut s = synced(&r);
    let (out, events, _) = run(
        &mut s,
        R::SetSettings {
            offset_ms: Some(-15),
            replace: None,
            quantize_on_input: None,
            quantize_strength: None,
        },
    );
    assert_eq!(
        out,
        Output::Record(RO::SetSettings {
            previous_offset_ms: 0,
            previous_replace: false,
            previous_quantize_on_input: project::InputGrid::None,
            previous_quantize_strength: 100,
        })
    );
    assert_eq!(
        events,
        vec![ProjectEvent::RecordSettingsChanged {
            offset_ms: -15,
            replace: false,
            quantize_on_input: project::InputGrid::None,
            quantize_strength: 100
        }]
    );
    assert_eq!(s.state.session.record.offset_ms, -15);
    assert!(s.state.history.undo.len() == r.state.history.undo.len());
    for bad in [-501, 501] {
        assert!(matches!(
            refused(
                &mut s,
                R::SetSettings {
                    offset_ms: Some(bad),
                    replace: None,
                    quantize_on_input: None,
                    quantize_strength: None
                }
            ),
            CommandError::Invalid { .. }
        ));
    }
    let (out, _, _) = run(
        &mut s,
        R::Start {
            target: None,
            latency_ms: Some(30.0),
        },
    );
    assert!(matches!(
        out,
        Output::Record(RO::Start { shift_ms, .. }) if shift_ms == 15.0
    ));
}

/// Commits a take from the examples' state; returns the state after, the output and the
/// events.
fn commit(
    r: &Rec,
    target: RecordTarget,
    shift_ms: f64,
    tracks: Vec<commands::record::TakeTrack>,
) -> (State, RO, Vec<ProjectEvent>) {
    let mut state = r.state.clone();
    let outcome = commands::run(
        &Command::Record(R::CommitTake {
            replace: None,
            quantize_on_input: None,
            quantize_strength: None,
            take: None,
            target,
            shift_ms,
            tracks,
        }),
        &mut state,
        &mut context(),
    )
    .unwrap();
    state.project.validate().unwrap();
    match outcome.output {
        Output::Record(o) => (state, o, outcome.events),
        o => panic!("{o:?}"),
    }
}

#[test]
fn an_overdub_lands_where_the_looping_grid_clip_played() {
    let r = rec();
    // The clip loops over one bar from tick 3840: tick 4800 is its position 960, tick
    // 8640 (two loops on) its position 960 too; tick 3000 is before the anchor: the loop's
    // formula gives 3000 - 3840 + 3840 = 3000.
    let (state, out, events) = commit(
        &r,
        RecordTarget::Scene {
            scene: r.scene.clone(),
        },
        0.0,
        take(
            &r,
            vec![
                note(4800, 60, 240),
                note(8640, 60, 240),
                note(3000, 61, 2000),
            ],
            Some(3840),
        ),
    );
    assert_eq!(
        out,
        RO::CommitTake {
            replaced: 0,
            clips: vec![TakeClip {
                track: r.track.clone(),
                clip: r.grid_clip.clone(),
                created: false,
                notes: 2,
            }],
            dropped: 0,
        }
    );
    let notes = notes_of(&state, &r.grid_clip);
    assert!(notes.contains(&Note::new(Tick(960), 60, Tick(240), 100)));
    // The note past the clip's end is cut there.
    assert!(notes.contains(&Note::new(Tick(3000), 61, Tick(840), 100)));
    assert_eq!(
        notes.len(),
        4,
        "the two notes of the clip are kept: {notes:?}"
    );
    assert!(events.contains(&ProjectEvent::ClipChanged {
        clip: r.grid_clip.clone(),
        part: commands::contract::ClipPart::Content
    }));
    // Without an anchor the clip starts at the bar of the first note.
    let (state, _, _) = commit(
        &r,
        RecordTarget::Scene {
            scene: r.scene.clone(),
        },
        0.0,
        take(&r, vec![note(4900, 60, 100)], None),
    );
    assert!(notes_of(&state, &r.grid_clip).contains(&Note::new(Tick(1060), 60, Tick(100), 100)));
}

#[test]
fn an_empty_cell_gets_a_looping_clip_from_the_bar_of_the_first_note() {
    let r = rec();
    // At 120 BPM a millisecond is 1.92 ticks: a shift of 50 ms moves every note 96 earlier.
    let (state, out, events) = commit(
        &r,
        RecordTarget::Scene {
            scene: r.empty.clone(),
        },
        50.0,
        take(&r, vec![note(4096, 60, 240), note(9096, 62, 240)], None),
    );
    let RO::CommitTake { clips, dropped, .. } = out else {
        panic!()
    };
    assert_eq!(dropped, 0);
    assert_eq!(clips.len(), 1);
    assert!(clips[0].created);
    let clip = &state.project.tracks[&r.track].clips[&clips[0].clip];
    assert_eq!(clip.slot.as_ref(), Some(&r.empty));
    let ClipContent::Midi {
        length,
        looping,
        notes,
    } = &clip.content
    else {
        panic!()
    };
    // From bar 1 (3840) to the bar after 9240: two bars, looping over them.
    assert_eq!(*length, Tick(7680));
    assert_eq!(
        *looping,
        Some(TickLoop {
            start: Tick(0),
            end: Tick(7680)
        })
    );
    assert_eq!(
        notes.iter().copied().collect::<Vec<_>>(),
        vec![
            Note::new(Tick(160), 60, Tick(240), 100),
            Note::new(Tick(5160), 62, Tick(240), 100),
        ]
    );
    assert!(
        matches!(&events[0], ProjectEvent::ClipCreated { placements, .. } if placements.is_empty())
    );
}

#[test]
fn the_arrangement_puts_each_note_where_it_was_played() {
    let r = rec();
    // 8000 is inside the placement of bar 3 (7680 to 11520); 1000 and 5000 are in the gap
    // before it; 12000 is after it; -10 is before the timeline.
    let (state, out, _) = commit(
        &r,
        RecordTarget::Arrangement {},
        0.0,
        take(
            &r,
            vec![
                note(8000, 60, 240),
                note(1000, 62, 240),
                note(5000, 64, 4000),
                note(12000, 65, 240),
                note(-300, 66, 240),
                note(-10, 67, 240),
                note(-200, 68, 100),
                note(-300, 69, 400),
            ],
            None,
        ),
    );
    let RO::CommitTake { clips, dropped, .. } = out else {
        panic!()
    };
    assert_eq!(
        dropped, 1,
        "the note that ends before tick 0, more than a sixteenth before it"
    );
    assert_eq!(clips.len(), 3, "{clips:?}");
    assert!(notes_of(&state, &r.placed_clip).contains(&Note::new(Tick(320), 60, Tick(240), 100)));
    let track = &state.project.tracks[&r.track];
    let placed: Vec<_> = clips
        .iter()
        .filter(|c| c.created)
        .map(|c| {
            let p = track
                .placements
                .values()
                .find(|p| p.clip == c.clip)
                .unwrap();
            (p.start, p.span, notes_of(&state, &c.clip))
        })
        .collect();
    // The gap before the placement: from bar 0, cut at its start (the long note too).
    assert_eq!(placed[0].0, Tick(0));
    assert_eq!(
        placed[0].1,
        ::project::Span::Ticks {
            length: Tick(7680),
            offset: Tick(0)
        }
    );
    assert_eq!(
        placed[0].2,
        vec![
            // Played at most a sixteenth before tick 0, across it or ended before it (#723):
            // moved there, with its length.
            Note::new(Tick(0), 67, Tick(240), 100),
            Note::new(Tick(0), 68, Tick(100), 100),
            // Played earlier, across tick 0: it starts there, cut.
            Note::new(Tick(0), 69, Tick(100), 100),
            Note::new(Tick(1000), 62, Tick(240), 100),
            Note::new(Tick(5000), 64, Tick(2680), 100),
        ]
    );
    // After it: from its end (bar 3 ends at 11520), one bar.
    assert_eq!(placed[1].0, Tick(11520));
    assert_eq!(placed[1].2, vec![Note::new(Tick(480), 65, Tick(240), 100)]);
}

#[test]
fn a_take_is_one_history_entry_that_one_undo_removes_and_redo_brings_back() {
    let r = rec();
    let mut s = synced(&r);
    run(
        &mut s,
        R::Start {
            target: None,
            latency_ms: None,
        },
    );
    run(&mut s, R::Stop {});
    let before = s.state.clone();
    let entries = s.state.history.undo.len();
    let (out, events, _) = run(
        &mut s,
        R::CommitTake {
            replace: None,
            quantize_on_input: None,
            quantize_strength: None,
            take: Some(1),
            target: RecordTarget::Scene {
                scene: r.empty.clone(),
            },
            shift_ms: 0.0,
            tracks: take(&r, vec![note(0, 60, 240), note(480, 62, 240)], None),
        },
    );
    assert!(matches!(out, Output::Record(RO::CommitTake { .. })));
    assert!(events.contains(&ProjectEvent::RecordingChanged {
        take: 1,
        state: TakeState::Committed
    }));
    assert!(s.ctx.engine.take.is_none(), "the next take may start");
    assert_eq!(s.state.history.undo.len(), entries + 1);
    let after = s.state.clone();
    s.run(&Command::History(history::Command::Undo {})).unwrap();
    assert_eq!(s.state.project, before.project);
    s.run(&Command::History(history::Command::Redo {})).unwrap();
    assert_eq!(s.state.project, after.project);
}

#[test]
fn a_take_with_nothing_to_write_changes_nothing() {
    let r = rec();
    let (state, out, events) = commit(
        &r,
        RecordTarget::Arrangement {},
        0.0,
        take(&r, vec![note(-500, 60, 100)], None),
    );
    assert_eq!(state.project, r.state.project);
    assert!(events.is_empty());
    assert!(matches!(out, RO::CommitTake { dropped: 1, .. }));
}

#[test]
fn a_bad_take_or_restore_is_refused_and_changes_nothing() {
    let r = rec();
    let mut s = synced(&r);
    for c in [
        R::CommitTake {
            replace: None,
            quantize_on_input: None,
            quantize_strength: None,
            take: None,
            target: RecordTarget::Arrangement {},
            shift_ms: f64::NAN,
            tracks: take(&r, vec![note(0, 60, 100)], None),
        },
        R::CommitTake {
            replace: None,
            quantize_on_input: None,
            quantize_strength: None,
            take: None,
            target: RecordTarget::Arrangement {},
            shift_ms: 0.0,
            tracks: take(&r, vec![note(0, 60, 0)], None),
        },
        R::CommitTake {
            replace: None,
            quantize_on_input: None,
            quantize_strength: None,
            take: None,
            target: RecordTarget::Arrangement {},
            shift_ms: 0.0,
            tracks: [take(&r, vec![], None), take(&r, vec![], None)].concat(),
        },
        R::CommitTake {
            replace: None,
            quantize_on_input: None,
            quantize_strength: None,
            take: None,
            target: RecordTarget::Arrangement {},
            shift_ms: 0.0,
            tracks: take(&r, vec![note(0, 60, i64::MAX)], None),
        },
        R::CommitTake {
            replace: None,
            quantize_on_input: None,
            quantize_strength: None,
            take: None,
            target: RecordTarget::Arrangement {},
            shift_ms: 0.0,
            tracks: take(&r, vec![note(i64::MIN + 1, 60, 10)], None),
        },
        R::Restore { clips: vec![] },
        // A note outside its clip.
        R::Restore {
            clips: vec![commands::record::ClipRestore {
                track: r.track.clone(),
                clip: r.grid_clip.clone(),
                state: Some(commands::record::RestoredClip {
                    snapshot: commands::clip::ClipSnapshot {
                        name: "X".into(),
                        color: None,
                        slot: Some(r.scene.clone()),
                        launch_quantize: None,
                        length: Tick(100),
                        looping: None,
                        notes: vec![Note::new(Tick(200), 60, Tick(10), 100)],
                    },
                    placements: vec![],
                }),
            }],
        },
    ] {
        let before = s.state.clone();
        refused(&mut s, c);
        assert_eq!(s.state, before);
    }
}

#[test]
fn a_commit_without_the_take_number_leaves_the_take_waiting() {
    let r = rec();
    let mut s = synced(&r);
    run(
        &mut s,
        R::Start {
            target: None,
            latency_ms: None,
        },
    );
    run(&mut s, R::Stop {});
    // An agent's own notes, or a redo of an older take: the waiting take stays.
    for take in [None, Some(7)] {
        let (_, events, _) = run(
            &mut s,
            R::CommitTake {
                replace: None,
                quantize_on_input: None,
                quantize_strength: None,
                take,
                target: RecordTarget::Arrangement {},
                shift_ms: 0.0,
                tracks: take_notes(&r),
            },
        );
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, ProjectEvent::RecordingChanged { .. }))
        );
        assert!(s.ctx.engine.take.is_some());
    }
}

fn take_notes(r: &Rec) -> Vec<commands::record::TakeTrack> {
    take(r, vec![note(100, 60, 100)], None)
}

#[test]
fn a_first_note_played_just_before_the_bar_starts_the_clip_on_that_bar() {
    let r = rec();
    // 7660 is 20 ticks before bar 2 (7680), within a sixteenth: the clip starts at bar 2 and the
    // note on its first tick; one bar long.
    let (state, out, _) = commit(
        &r,
        RecordTarget::Scene {
            scene: r.empty.clone(),
        },
        0.0,
        take(&r, vec![note(7660, 60, 240), note(8640, 62, 240)], None),
    );
    let RO::CommitTake { clips, dropped, .. } = out else {
        panic!()
    };
    assert_eq!(dropped, 0);
    let clip = &state.project.tracks[&r.track].clips[&clips[0].clip];
    let ClipContent::Midi { length, notes, .. } = &clip.content else {
        panic!()
    };
    assert_eq!(*length, Tick(3840));
    assert_eq!(
        notes.iter().copied().collect::<Vec<_>>(),
        vec![
            Note::new(Tick(0), 60, Tick(240), 100),
            Note::new(Tick(960), 62, Tick(240), 100),
        ]
    );
    // In the arrangement too: the new clip is placed on bar 2 (the gap before the clip of bar 3).
    let (state, out, _) = commit(
        &r,
        RecordTarget::Arrangement {},
        0.0,
        take(&r, vec![note(3830, 60, 240)], None),
    );
    let RO::CommitTake { clips, .. } = out else {
        panic!()
    };
    let p = state.project.tracks[&r.track]
        .placements
        .values()
        .find(|p| p.clip == clips[0].clip)
        .unwrap();
    assert_eq!(p.start, Tick(3840));
}

/// One target's result: its name, the clip's start (arrangement only) and the notes.
type Landed = (&'static str, Option<i64>, Vec<(u8, i64)>);

/// Where `played` (ticks around bar 2, 7680, at `bpm`; 1.92 ticks a millisecond at 120 BPM)
/// lands in the three targets of a take (#732): an empty cell, the clip of a cell without an
/// anchor, and a new clip of the arrangement (the same notes one bar earlier, around 3840, whose
/// placement start is given back on the scale of the others, plus 3840). Each gives the notes'
/// `(pitch, position)` in their clip, which tells the bar the clip counts from.
fn downbeat_targets(bpm: f64, played: &[(i64, u8)]) -> Vec<Landed> {
    let mut r = rec();
    commands::run(
        &Command::Transport(transport::Command::SetTempo {
            bpm,
            placements: None,
        }),
        &mut r.state,
        &mut context(),
    )
    .unwrap();
    let take_of = |shift: i64| {
        take(
            &r,
            played
                .iter()
                .map(|&(start, pitch)| note(start - shift, pitch, 80))
                .collect(),
            None,
        )
    };
    let pitches: Vec<u8> = played.iter().map(|&(_, p)| p).collect();
    let positions = |notes: &[Note]| {
        let mut out: Vec<_> = notes
            .iter()
            .filter(|n| pitches.contains(&n.pitch))
            .map(|n| (n.pitch, n.start.get()))
            .collect();
        out.sort();
        out
    };
    let mut out = Vec::new();

    let (state, ro, _) = commit(
        &r,
        RecordTarget::Scene {
            scene: r.empty.clone(),
        },
        0.0,
        take_of(0),
    );
    let RO::CommitTake { clips, dropped, .. } = ro else {
        panic!()
    };
    assert_eq!(dropped, 0);
    out.push((
        "empty cell",
        None,
        positions(&notes_of(&state, &clips[0].clip)),
    ));

    let (state, _, _) = commit(
        &r,
        RecordTarget::Scene {
            scene: r.scene.clone(),
        },
        0.0,
        take_of(0),
    );
    out.push((
        "cell without an anchor",
        None,
        positions(&notes_of(&state, &r.grid_clip)),
    ));

    let (state, ro, _) = commit(&r, RecordTarget::Arrangement {}, 0.0, take_of(3840));
    let RO::CommitTake { clips, .. } = ro else {
        panic!()
    };
    let p = state.project.tracks[&r.track]
        .placements
        .values()
        .find(|p| p.clip == clips[0].clip)
        .unwrap();
    out.push((
        "arrangement",
        Some(p.start.get() + 3840),
        positions(&notes_of(&state, &clips[0].clip)),
    ));
    out
}

/// `played` lands in every target with the clip counting from `origin` and the notes at
/// `expected` in it.
fn assert_downbeat(played: &[(i64, u8)], origin: i64, expected: &[(u8, i64)]) {
    assert_downbeat_at(120.0, played, origin, expected);
}

/// [`assert_downbeat`] at `bpm`.
fn assert_downbeat_at(bpm: f64, played: &[(i64, u8)], origin: i64, expected: &[(u8, i64)]) {
    for (target, start, notes) in downbeat_targets(bpm, played) {
        if let Some(start) = start {
            assert_eq!(start, origin, "{target}: clip start");
        }
        assert_eq!(notes, expected, "{target}: notes");
    }
}

/// #732: a chord played early on the downbeat (notes 0, 10 and 25 ms after the first, all in
/// the sixteenth before the bar) starts the clip on the bar, each note on its first tick with
/// its length; a note after the bar stays as played.
#[test]
fn an_early_downbeat_chord_starts_the_clip_on_the_bar() {
    // 7580 is 100 ticks (52 ms) before the bar; +19 ticks is 9.9 ms, +48 is 25 ms.
    assert_downbeat(
        &[(7580, 60), (7599, 64), (7628, 67), (7870, 72)],
        7680,
        &[(60, 0), (64, 0), (67, 0), (72, 190)],
    );
}

/// #732: a single downbeat played early is put on the bar.
#[test]
fn a_single_early_downbeat_starts_the_clip_on_the_bar() {
    assert_downbeat(&[(7660, 60), (8640, 62)], 7680, &[(60, 0), (62, 960)]);
}

/// #732: a pickup before an early downbeat (in the same sixteenth, 94 ms apart): nothing is
/// moved, the clip starts on the bar before and both notes keep the place they were heard at.
#[test]
fn a_pickup_before_the_downbeat_keeps_every_note_where_it_was_heard() {
    assert_downbeat(&[(7480, 59), (7660, 60)], 3840, &[(59, 3640), (60, 3820)]);
}

/// #732: a second note 31 ms after the first (60 ticks, 31.25 ms) is not the same chord:
/// nothing is moved.
#[test]
fn a_note_31_ms_after_the_first_is_not_the_downbeat_chord() {
    assert_downbeat(&[(7580, 60), (7640, 64)], 3840, &[(60, 3740), (64, 3800)]);
}

/// #732: the chord is measured in time, not ticks. At 125 BPM (2 ticks a millisecond) 59 ticks
/// is 29.5 ms, a chord, and 60 ticks is 30 ms, not one; at 999 BPM (16 ticks a millisecond) the
/// whole sixteenth lasts 15 ms, so two notes 200 ticks apart are still one downbeat, and so is
/// the issue's batch, notes 10 ms (160 ticks) apart.
#[test]
fn the_downbeat_chord_is_measured_in_milliseconds_at_any_tempo() {
    assert_downbeat_at(125.0, &[(7580, 60), (7639, 64)], 7680, &[(60, 0), (64, 0)]);
    assert_downbeat_at(
        125.0,
        &[(7580, 60), (7640, 64)],
        3840,
        &[(60, 3740), (64, 3800)],
    );
    assert_downbeat_at(999.0, &[(7450, 60), (7650, 64)], 7680, &[(60, 0), (64, 0)]);
    assert_downbeat_at(
        999.0,
        &[(7450, 60), (7610, 61), (7770, 62)],
        7680,
        &[(60, 0), (61, 0), (62, 90)],
    );
}

/// #560: the engine applies the take's shift (the latency plus the session's offset), the
/// same one `record.start` answers.
#[test]
fn the_take_s_shift_goes_to_the_engine_as_record_start_answers_it() {
    for (latency_ms, offset_ms, shift_us) in [
        (Some(42.0), 8, 50_000),
        (None, -500, -500_000),
        (Some(1000.0), 500, 1_500_000),
        (Some(20.4), 0, 20_400),
    ] {
        let r = rec();
        let mut s = synced(&r);
        run(
            &mut s,
            R::SetSettings {
                offset_ms: Some(offset_ms),
                replace: None,
                quantize_on_input: None,
                quantize_strength: None,
            },
        );
        let (out, _, control) = run(
            &mut s,
            R::Start {
                target: None,
                latency_ms,
            },
        );
        let Output::Record(RO::Start { shift_ms, .. }) = out else {
            panic!("{out:?}")
        };
        assert_eq!(f64::from(shift_us) / 1000.0, shift_ms);
        assert!(
            control.contains(&ControlMsg::SetRecording {
                recording: true,
                shift_us
            }),
            "{control:?}"
        );
    }
}
