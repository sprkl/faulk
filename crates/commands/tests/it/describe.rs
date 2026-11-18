//! What the history says an entry acted on (#473): the names the panel's labels are built
//! from ("Mute Lead", "Delete 17 clips (Bass, Drums, Lead)"), read before the command
//! applies, merged across a group, kept by undo and redo.

use ::project::{PlacementId, Tick, TrackId};
use commands::clip::ClipTarget;
use commands::describe::Subject;
use commands::testing::{context, fixture};
use commands::{Command, Context, Output, State, arrangement, clip, history, mixer, run, track};

fn create_track(s: &mut State, ctx: &mut Context, name: &str) -> TrackId {
    let out = run(
        &Command::Track(track::Command::Create {
            kind: ::project::TrackKind::Midi,
            instrument: None,
            name: Some(name.into()),
            color: None,
        }),
        s,
        ctx,
    )
    .unwrap();
    let Output::Track(track::Output::Create { track, .. }) = out.output else {
        panic!()
    };
    track
}

fn placed_clip(s: &mut State, ctx: &mut Context, track: &TrackId, start: i64) -> PlacementId {
    let out = run(
        &Command::Clip(clip::Command::Create {
            track: track.clone(),
            at: ClipTarget::Arrangement { start: Tick(start) },
            length: None,
            name: None,
        }),
        s,
        ctx,
    )
    .unwrap();
    let Output::Clip(clip::Output::Create { placement, .. }) = out.output else {
        panic!()
    };
    placement.unwrap()
}

fn top(s: &State) -> (&'static str, Subject, u32) {
    let entry = s.history.undo.last().unwrap();
    (entry.name, entry.subject.clone().unwrap(), entry.others)
}

#[test]
fn a_mute_names_its_track_and_its_value() {
    let f = fixture();
    let mut s = f.state.clone();
    run(
        &Command::Mixer(mixer::Command::SetMute {
            track: f.track.clone(),
            mute: true,
        }),
        &mut s,
        &mut context(),
    )
    .unwrap();
    let (name, subject, others) = top(&s);
    assert_eq!(name, "mixer.set_mute");
    assert_eq!(subject.objects, ["Synth"]);
    assert_eq!(subject.on, Some(true));
    assert_eq!((subject.count, others), (1, 0));
    // The panel reads it from the view.
    let view = s.history.view();
    assert_eq!(view.undo.last().unwrap().subject, Some(subject));
}

#[test]
fn a_delete_of_many_placements_counts_them_and_names_their_tracks() {
    let f = fixture();
    let mut s = f.state.clone();
    let mut ctx = context();
    let mut all = Vec::new();
    for name in ["Bass", "Drums", "Lead"] {
        let t = create_track(&mut s, &mut ctx, name);
        for i in 0..3 {
            all.push(placed_clip(&mut s, &mut ctx, &t, i * 3840));
        }
    }
    // A created clip is named from the project after the command (it had no name before).
    let (name, subject, _) = top(&s);
    assert_eq!(name, "clip.create");
    assert_eq!(subject.objects, ["Lead 3"]);
    assert_eq!(subject.within, ["Lead"]);
    run(
        &Command::Arrangement(arrangement::Command::Delete {
            placements: all.clone(),
        }),
        &mut s,
        &mut ctx,
    )
    .unwrap();
    let (name, subject, _) = top(&s);
    assert_eq!(name, "arrangement.delete");
    assert_eq!(subject.count, 9);
    assert_eq!(subject.within, ["Bass", "Drums", "Lead"]);
    assert_eq!(subject.objects.len(), 8, "at most eight names are kept");
    // Undo and redo keep the words with the entry.
    run(
        &Command::History(history::Command::Undo {}),
        &mut s,
        &mut ctx,
    )
    .unwrap();
    assert_eq!(
        s.history.redo.last().unwrap().subject.as_ref(),
        Some(&subject)
    );
    run(
        &Command::History(history::Command::Redo {}),
        &mut s,
        &mut ctx,
    )
    .unwrap();
    assert_eq!(top(&s).1, subject);
}

#[test]
fn a_group_merges_the_commands_of_its_first_name_and_counts_the_others() {
    let f = fixture();
    let mut s = f.state.clone();
    let mut ctx = context();
    let bass = create_track(&mut s, &mut ctx, "Bass");
    let drums = create_track(&mut s, &mut ctx, "Drums");
    run(
        &Command::History(history::Command::BeginGroup {}),
        &mut s,
        &mut ctx,
    )
    .unwrap();
    for t in [&bass, &drums, &f.track] {
        run(
            &Command::Mixer(mixer::Command::SetMute {
                track: t.clone(),
                mute: true,
            }),
            &mut s,
            &mut ctx,
        )
        .unwrap();
    }
    // An edit of another name joins the entry as one of its "others".
    run(
        &Command::Track(track::Command::Rename {
            track: bass.clone(),
            name: "Sub".into(),
        }),
        &mut s,
        &mut ctx,
    )
    .unwrap();
    run(
        &Command::History(history::Command::EndGroup {}),
        &mut s,
        &mut ctx,
    )
    .unwrap();
    let (name, subject, others) = top(&s);
    assert_eq!(name, "mixer.set_mute");
    assert_eq!(subject.count, 3);
    assert_eq!(subject.objects, ["Bass", "Drums", "Synth"]);
    assert_eq!(subject.on, Some(true));
    assert_eq!(others, 1);
    // A mute and an unmute in one group say no single value.
    let keys = create_track(&mut s, &mut ctx, "Keys");
    run(
        &Command::History(history::Command::BeginGroup {}),
        &mut s,
        &mut ctx,
    )
    .unwrap();
    for (t, mute) in [(&bass, false), (&keys, true)] {
        run(
            &Command::Mixer(mixer::Command::SetMute {
                track: t.clone(),
                mute,
            }),
            &mut s,
            &mut ctx,
        )
        .unwrap();
    }
    run(
        &Command::History(history::Command::EndGroup {}),
        &mut s,
        &mut ctx,
    )
    .unwrap();
    assert_eq!(top(&s).1.on, None);
}

#[test]
fn a_rename_says_the_old_and_the_new_name() {
    let f = fixture();
    let mut s = f.state.clone();
    run(
        &Command::Track(track::Command::Rename {
            track: f.track.clone(),
            name: "  Keys ".into(),
        }),
        &mut s,
        &mut context(),
    )
    .unwrap();
    let (_, subject, _) = top(&s);
    assert_eq!(subject.objects, ["Synth"]);
    assert_eq!(subject.to.as_deref(), Some("Keys"));
}

#[test]
fn a_refused_command_records_nothing_and_changes_nothing() {
    let f = fixture();
    let mut s = f.state.clone();
    let before = s.clone();
    let missing = TrackId::generate(&mut ::project::IdGenerator::for_tests());
    run(
        &Command::Mixer(mixer::Command::SetMute {
            track: missing,
            mute: true,
        }),
        &mut s,
        &mut context(),
    )
    .unwrap_err();
    assert_eq!(s, before);
}

#[test]
fn a_take_is_one_entry_that_counts_its_notes_in_its_tracks() {
    use commands::record::{Command as R, RecordTarget, TakeNote, TakeTrack};
    let f = fixture();
    let mut s = f.state.clone();
    let mut ctx = context();
    let bass = create_track(&mut s, &mut ctx, "Bass");
    let note = |start| TakeNote {
        start: Tick(start),
        length: Tick(240),
        pitch: 60,
        velocity: 100,
    };
    let entries = s.history.undo.len();
    run(
        &Command::Record(R::CommitTake {
            replace: None,
            quantize_on_input: None,
            quantize_strength: None,
            take: None,
            target: RecordTarget::Arrangement {},
            shift_ms: 0.0,
            tracks: vec![
                TakeTrack {
                    track: f.track.clone(),
                    notes: vec![note(0), note(480), note(960)],
                    anchor: None,
                },
                TakeTrack {
                    track: bass,
                    notes: vec![note(0)],
                    anchor: None,
                },
            ],
        }),
        &mut s,
        &mut ctx,
    )
    .unwrap();
    assert_eq!(s.history.undo.len(), entries + 1, "one take, one entry");
    let (name, subject, others) = top(&s);
    assert_eq!(name, "record.commit_take");
    assert_eq!(subject.count, 4);
    assert_eq!(subject.within, ["Synth", "Bass"]);
    assert_eq!(others, 0);
}

/// Runs a command that must apply.
fn go(s: &mut State, ctx: &mut Context, command: Command) -> Output {
    run(&command, s, ctx).unwrap().output
}

#[test]
fn a_track_with_no_name_is_said_by_its_position_and_its_instrument() {
    let f = fixture();
    let mut s = f.state.clone();
    let mut ctx = context();
    for _ in 0..2 {
        go(
            &mut s,
            &mut ctx,
            Command::Track(track::Command::Create {
                kind: ::project::TrackKind::Midi,
                instrument: None,
                name: None,
                color: None,
            }),
        );
    }
    let (name, subject, _) = top(&s);
    assert_eq!(name, "track.create");
    assert!(subject.objects.is_empty(), "no name to say");
    assert_eq!(subject.unnamed, [3], "the third track of the project");
    assert_eq!(subject.within, ["Oscillator"], "the instrument it plays");
    // A clip made on it names its track by position, never by an empty name.
    let track = s.project.tracks_in_order()[2].0.clone();
    go(
        &mut s,
        &mut ctx,
        Command::Clip(clip::Command::Create {
            track,
            at: ClipTarget::Arrangement { start: Tick(0) },
            length: None,
            name: None,
        }),
    );
    let (name, subject, _) = top(&s);
    assert_eq!(name, "clip.create");
    assert!(subject.within.is_empty());
    assert_eq!(subject.within_unnamed, [3]);
    assert_eq!(subject.objects.len(), 1, "the clip has a name");
}

#[test]
fn a_scene_is_said_by_its_name_or_its_position() {
    use commands::scene;
    let f = fixture();
    let mut s = f.state.clone();
    let mut ctx = context();
    for name in [None, Some("Chorus".to_owned())] {
        go(
            &mut s,
            &mut ctx,
            Command::Scene(scene::Command::Create { name }),
        );
    }
    let (name, subject, _) = top(&s);
    assert_eq!(name, "scene.create");
    assert_eq!(subject.objects, ["Chorus"]);
    let first = s.project.scenes_in_order()[0].0.clone();
    go(
        &mut s,
        &mut ctx,
        Command::Scene(scene::Command::Delete { scene: first }),
    );
    let (name, subject, _) = top(&s);
    assert_eq!(name, "scene.delete");
    assert!(subject.objects.is_empty());
    assert_eq!(subject.unnamed, [1]);
}

#[test]
fn an_automation_entry_names_its_parameter_and_a_parameter_change_its_value() {
    use commands::{automation, device};
    let f = fixture();
    let mut s = f.state.clone();
    let mut ctx = context();
    let Output::Automation(automation::Output::LaneCreate { lane }) = go(
        &mut s,
        &mut ctx,
        Command::Automation(automation::Command::LaneCreate {
            track: Some(f.track.clone()),
            master: false,
            target: ::project::AutomationTarget::Device {
                device: f.device.clone(),
                param: "attack".into(),
            },
        }),
    ) else {
        panic!()
    };
    let param = |subject: &Subject| {
        let p = subject.param.clone().unwrap();
        (p.kind, p.device, p.key)
    };
    let expected = (
        Some("faulk.osc_synth".to_owned()),
        Some("Oscillator".to_owned()),
        "attack".to_owned(),
    );
    let (name, subject, _) = top(&s);
    assert_eq!(name, "automation.lane.create");
    assert_eq!(param(&subject), expected);
    assert_eq!(subject.within, ["Synth"]);
    let point = |tick| automation::Point {
        tick: Tick(tick),
        value: 0.5,
        shape: ::project::Shape::Linear,
    };
    go(
        &mut s,
        &mut ctx,
        Command::Automation(automation::Command::PointsAdd {
            lane: lane.clone(),
            points: vec![point(0), point(960)],
        }),
    );
    let (name, subject, _) = top(&s);
    assert_eq!((name, subject.count), ("automation.points.add", 2));
    assert_eq!(param(&subject), expected);
    // A ramp is one `points.restore`: it names the lane too.
    go(
        &mut s,
        &mut ctx,
        Command::Automation(automation::Command::PointsRestore {
            lane,
            points: vec![point(0), point(1920)],
            remove: vec![],
        }),
    );
    let (name, subject, _) = top(&s);
    assert_eq!((name, subject.count), ("automation.points.restore", 2));
    assert_eq!(param(&subject), expected);
    go(
        &mut s,
        &mut ctx,
        Command::Device(device::Command::SetParam {
            device: f.device.clone(),
            param: "attack".into(),
            value: 0.25,
        }),
    );
    let (name, subject, _) = top(&s);
    assert_eq!(name, "device.set_param");
    assert_eq!(param(&subject), expected);
    assert_eq!(subject.value, Some(0.25));
}

#[test]
fn an_unnamed_track_is_said_by_its_position_when_deleted_or_renamed() {
    let f = fixture();
    let mut s = f.state.clone();
    let mut ctx = context();
    for _ in 0..2 {
        go(
            &mut s,
            &mut ctx,
            Command::Track(track::Command::Create {
                kind: ::project::TrackKind::Audio,
                instrument: None,
                name: None,
                color: None,
            }),
        );
    }
    let second = s.project.tracks_in_order()[1].0.clone();
    go(
        &mut s,
        &mut ctx,
        Command::Track(track::Command::Rename {
            track: second,
            name: "Pad".into(),
        }),
    );
    let (name, subject, _) = top(&s);
    assert_eq!(name, "track.rename");
    assert_eq!(subject.unnamed, [2]);
    assert_eq!(subject.to.as_deref(), Some("Pad"));
    let third = s.project.tracks_in_order()[2].0.clone();
    go(
        &mut s,
        &mut ctx,
        Command::Track(track::Command::Delete { track: third }),
    );
    let (name, subject, _) = top(&s);
    assert_eq!(name, "track.delete");
    assert_eq!(subject.unnamed, [3]);
}

#[test]
fn a_group_keeps_the_parameter_only_when_every_command_agrees() {
    use commands::device;
    let f = fixture();
    let mut s = f.state.clone();
    let mut ctx = context();
    go(
        &mut s,
        &mut ctx,
        Command::History(history::Command::BeginGroup {}),
    );
    for value in [0.1, 0.2] {
        go(
            &mut s,
            &mut ctx,
            Command::Device(device::Command::SetParam {
                device: f.device.clone(),
                param: "attack".into(),
                value,
            }),
        );
    }
    go(
        &mut s,
        &mut ctx,
        Command::History(history::Command::EndGroup {}),
    );
    let (name, subject, _) = top(&s);
    assert_eq!(name, "device.set_param");
    assert_eq!(subject.count, 2);
    assert_eq!(subject.param.as_ref().unwrap().key, "attack");
    assert_eq!(subject.value, None, "two values, none said");
}
