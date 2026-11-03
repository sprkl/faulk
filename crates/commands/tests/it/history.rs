//! The undo history: `run` records, `history.*` undo, redo and group. One test per state
//! transition of the history (record, merge, redo dropped, no-op, live, irreversible,
//! empty stacks, groups, failure, author).

use commands::history::Entry;
use commands::testing::{check_refusal, context, fixture};
use commands::{
    Author, Command, CommandError, Context, GroupId, History, Kind, Meta, ProjectEvent, State,
    Undo, UndoKind, device, history, live, project, run,
};

fn rename(name: &str) -> Command {
    Command::Project(project::Command::Rename { name: name.into() })
}

fn set(device: &::project::DeviceId, param: &str, value: f64) -> Command {
    Command::Device(device::Command::SetParam {
        device: device.clone(),
        param: param.into(),
        value,
    })
}

fn h(c: history::Command) -> Command {
    Command::History(c)
}

fn agent(name: &str) -> Context {
    let mut c = context();
    c.author = Author::Agent { name: name.into() };
    c
}

fn param(s: &State, d: &::project::DeviceId, p: &str) -> f64 {
    s.project
        .tracks
        .values()
        .find_map(|t| t.devices.get(d))
        .unwrap()
        .params[p]
}

#[test]
fn an_edit_is_recorded_with_its_author_and_undo_gives_the_project_back() {
    let f = fixture();
    let mut s = f.state.clone();
    let before = s.project.clone();
    let out = run(
        &set(&f.device, "attack", 0.25),
        &mut s,
        &mut agent("claude"),
    )
    .unwrap();
    assert_eq!(
        out.events.last(),
        Some(&ProjectEvent::HistoryChanged {
            can_undo: true,
            can_redo: false
        })
    );
    let top = s.history.undo.last().unwrap();
    assert_eq!(top.name, "device.set_param");
    assert_eq!(
        top.author,
        Author::Agent {
            name: "claude".into()
        }
    );
    let undo = run(&h(history::Command::Undo {}), &mut s, &mut context()).unwrap();
    assert_eq!(s.project, before);
    assert_eq!(
        undo.output,
        commands::Output::History(history::Output::Undo {
            name: Some("device.set_param".into()),
            author: Some(Author::Agent {
                name: "claude".into()
            }),
        })
    );
    // The redo entry keeps the author; redo brings the edit back.
    assert_eq!(
        s.history.redo.last().unwrap().author,
        Author::Agent {
            name: "claude".into()
        }
    );
    run(&h(history::Command::Redo {}), &mut s, &mut context()).unwrap();
    assert_eq!(param(&s, &f.device, "attack"), 0.25);
    assert!(s.history.redo.is_empty());
}

#[test]
fn undoing_twice_walks_back_through_the_fixture_rename() {
    let f = fixture();
    let mut s = f.state.clone();
    run(&rename("Second"), &mut s, &mut context()).unwrap();
    run(&h(history::Command::Undo {}), &mut s, &mut context()).unwrap();
    assert_eq!(s.project.name, "Demo");
    run(&h(history::Command::Undo {}), &mut s, &mut context()).unwrap();
    assert_eq!(s.project.name, "Untitled");
    assert_eq!(s.history.redo.len(), 2);
    run(&h(history::Command::Redo {}), &mut s, &mut context()).unwrap();
    run(&h(history::Command::Redo {}), &mut s, &mut context()).unwrap();
    assert_eq!(s.project.name, "Second");
}

#[test]
fn a_new_edit_drops_what_could_be_redone() {
    let f = fixture();
    let mut s = f.state.clone();
    run(&h(history::Command::Undo {}), &mut s, &mut context()).unwrap();
    assert_eq!(s.history.redo.len(), 1);
    run(&rename("Other"), &mut s, &mut context()).unwrap();
    assert!(s.history.redo.is_empty());
    let redo = run(&h(history::Command::Redo {}), &mut s, &mut context()).unwrap();
    assert_eq!(redo.undo, Undo::None, "nothing left to redo");
    assert_eq!(s.project.name, "Other");
}

#[test]
fn an_edit_that_changes_nothing_records_nothing_and_keeps_the_redo() {
    let f = fixture();
    let mut s = f.state.clone();
    run(&h(history::Command::Undo {}), &mut s, &mut context()).unwrap();
    let history = s.history.clone();
    let out = run(&rename("Untitled"), &mut s, &mut context()).unwrap();
    assert!(out.events.is_empty());
    assert_eq!(s.history, history);
}

#[test]
fn live_and_query_commands_leave_the_history_alone() {
    let f = fixture();
    let mut s = f.state.clone();
    run(&h(history::Command::Undo {}), &mut s, &mut context()).unwrap();
    let history = s.history.clone();
    for command in [
        Command::Live(live::Command::NoteOn {
            track: f.track.clone(),
            note: 60,
            velocity: 100,
        }),
        Command::Live(live::Command::AllNotesOff {}),
        Command::Live(live::Command::AllSoundOff {}),
        Command::Launch(commands::launch::Command::BackToArrangement {}),
        Command::Launch(commands::launch::Command::BackTrackToArrangement {
            track: f.track.clone(),
        }),
        Command::Midi(commands::midi::Command::ListInputs {}),
        Command::Midi(commands::midi::Command::SelectInput {
            id: Some("x".into()),
        }),
        Command::View(commands::view::Command::SelectTrack {
            track: Some(f.track.clone()),
        }),
    ] {
        run(&command, &mut s, &mut context()).unwrap();
        assert_eq!(s.history, history, "{}", command.name());
    }
}

#[test]
fn an_irreversible_edit_empties_both_stacks() {
    let f = fixture();
    let mut hist = f.state.history.clone();
    hist.redo.push(hist.undo[0].clone());
    let destructive = Meta {
        name: "track.delete",
        kind: Kind::Edit,
        undo: UndoKind::None,
        destructive: true,
    };
    assert!(hist.record(destructive, &Undo::None, &Author::User, None, 0, None));
    assert!(!hist.can_undo() && !hist.can_redo());
    // Nothing left to empty: the history does not change.
    assert!(!hist.record(destructive, &Undo::None, &Author::User, None, 0, None));
    // An edit without inverse that is not destructive (a no-op) records nothing.
    let mut kept = f.state.history.clone();
    assert!(!kept.record(
        rename("x").meta(),
        &Undo::None,
        &Author::User,
        None,
        0,
        None
    ));
    assert_eq!(kept, f.state.history);
}

#[test]
fn undo_and_redo_on_empty_stacks_change_nothing() {
    let mut s = commands::testing::state();
    let before = s.clone();
    for c in [history::Command::Undo {}, history::Command::Redo {}] {
        let out = run(&h(c), &mut s, &mut context()).unwrap();
        assert_eq!(out.undo, Undo::None);
        assert!(out.events.is_empty());
        assert_eq!(s, before);
    }
}

#[test]
fn a_group_is_one_entry_undone_in_reverse_order() {
    let f = fixture();
    let mut s = f.state.clone();
    let before = s.project.clone();
    let begin = run(&h(history::Command::BeginGroup {}), &mut s, &mut context()).unwrap();
    let commands::Output::History(history::Output::BeginGroup { group }) = begin.output else {
        panic!("{:?}", begin.output)
    };
    // A drag: the same parameter several times, then another one.
    for v in [0.1, 0.2, 0.3] {
        run(&set(&f.device, "attack", v), &mut s, &mut context()).unwrap();
    }
    run(&set(&f.device, "gain", 0.5), &mut s, &mut context()).unwrap();
    run(&h(history::Command::EndGroup {}), &mut s, &mut context()).unwrap();
    assert_eq!(
        s.history.undo.len(),
        2,
        "the fixture's rename, then the group"
    );
    let entry = s.history.undo.last().unwrap();
    assert_eq!(entry.group, Some(group));
    assert_eq!(entry.commands.len(), 4);
    run(&h(history::Command::Undo {}), &mut s, &mut context()).unwrap();
    assert_eq!(s.project, before, "every edit of the group undone at once");
    run(&h(history::Command::Redo {}), &mut s, &mut context()).unwrap();
    assert_eq!(param(&s, &f.device, "attack"), 0.3);
    assert_eq!(param(&s, &f.device, "gain"), 0.5);
    // After the group, an edit is a new entry.
    run(&set(&f.device, "gain", 0.6), &mut s, &mut context()).unwrap();
    assert_eq!(s.history.undo.len(), 3);
}

#[test]
fn the_context_group_merges_an_agent_turn_and_wins_over_the_open_group() {
    let f = fixture();
    let mut s = f.state.clone();
    let mut turn = agent("claude");
    turn.group = Some(s.history.new_group());
    run(&set(&f.device, "attack", 0.1), &mut s, &mut turn).unwrap();
    run(&rename("By agent"), &mut s, &mut turn).unwrap();
    assert_eq!(s.history.undo.len(), 2);
    assert_eq!(s.history.undo[1].commands.len(), 2);
    // The user opens a group; the agent's turn still goes to the turn's group.
    run(&h(history::Command::BeginGroup {}), &mut s, &mut context()).unwrap();
    run(&set(&f.device, "gain", 0.2), &mut s, &mut turn).unwrap();
    assert_eq!(s.history.undo[1].commands.len(), 3);
    assert_eq!(s.history.undo.len(), 2);
}

#[test]
fn another_authors_edit_does_not_join_an_open_group_nor_ends_it() {
    let f = fixture();
    let mut s = f.state.clone();
    run(
        &h(history::Command::BeginGroup {}),
        &mut s,
        &mut agent("claude"),
    )
    .unwrap();
    run(&set(&f.device, "attack", 0.1), &mut s, &mut agent("claude")).unwrap();
    run(&set(&f.device, "gain", 0.2), &mut s, &mut context()).unwrap();
    let top = s.history.undo.last().unwrap();
    assert_eq!(
        top.author,
        Author::User,
        "the user's edit has its own entry"
    );
    assert_eq!(top.group, None);
    let err = check_refusal(&h(history::Command::EndGroup {}), &s, &context()).unwrap();
    assert!(err.to_string().contains("agent:claude"), "{err}");
    run(
        &h(history::Command::EndGroup {}),
        &mut s,
        &mut agent("claude"),
    )
    .unwrap();
    assert_eq!(s.history.open_group, None);
}

#[test]
fn group_misuse_is_refused_and_leaves_the_state_untouched() {
    let f = fixture();
    let err = check_refusal(&h(history::Command::EndGroup {}), &f.state, &context()).unwrap();
    assert_eq!(err.code(), "invalid");
    let mut open = f.state.clone();
    run(
        &h(history::Command::BeginGroup {}),
        &mut open,
        &mut context(),
    )
    .unwrap();
    for c in [
        history::Command::BeginGroup {},
        history::Command::Undo {},
        history::Command::Redo {},
    ] {
        let err = check_refusal(&h(c), &open, &context()).unwrap();
        assert!(matches!(err, CommandError::Invalid { .. }), "{err}");
    }
}

#[test]
fn group_ids_are_unique_within_a_history() {
    let mut hist = History::default();
    let a = hist.new_group();
    let b = hist.new_group();
    assert_ne!(a, b);
    assert_eq!(a, GroupId(1));
}

#[test]
fn an_entry_that_fails_halfway_leaves_the_state_and_history_untouched() {
    let f = fixture();
    let mut s = f.state.clone();
    let missing = ::project::DeviceId::parse("01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap();
    // Applied in reverse order: the rename applies, then the unknown device fails.
    s.history.undo.push(Entry {
        id: 0,
        at: 0,
        name: "device.set_param",
        subject: None,
        others: 0,
        author: Author::User,
        group: Some(GroupId(99)),
        commands: vec![set(&missing, "attack", 0.1), rename("Half")],
    });
    let err = check_refusal(&h(history::Command::Undo {}), &s, &context()).unwrap();
    assert_eq!(err.code(), "not_found");
    // A single failing command: refused as well, the entry stays.
    s.history.undo.last_mut().unwrap().commands = vec![set(&missing, "attack", 0.1)];
    check_refusal(&h(history::Command::Undo {}), &s, &context()).unwrap();
}

#[test]
fn history_commands_are_never_recorded() {
    let f = fixture();
    let mut s = f.state.clone();
    run(&h(history::Command::Undo {}), &mut s, &mut context()).unwrap();
    assert!(s.history.undo.is_empty());
    run(&h(history::Command::Redo {}), &mut s, &mut context()).unwrap();
    assert_eq!(s.history.undo.len(), 1);
    assert!(s.history.redo.is_empty());
}

#[test]
fn the_author_is_a_string_in_the_output_schema() {
    let tool = commands::catalog()
        .into_iter()
        .find(|t| t.name == "history.undo")
        .unwrap();
    let author = &tool.output_schema["$defs"]["Author"];
    assert_eq!(author["type"], "string", "{author}");
    let out = serde_json::to_value(history::Output::Undo {
        name: Some("project.rename".into()),
        author: Some(Author::Agent { name: "a".into() }),
    })
    .unwrap();
    assert_eq!(out["author"], "agent:a");
}

#[test]
fn another_authors_edit_inside_a_group_splits_it_in_application_order() {
    let f = fixture();
    let mut s = f.state.clone();
    let mut turn = agent("claude");
    turn.group = Some(s.history.new_group());
    run(&set(&f.device, "attack", 0.1), &mut s, &mut turn).unwrap();
    run(&set(&f.device, "gain", 0.2), &mut s, &mut context()).unwrap();
    run(&rename("By agent"), &mut s, &mut turn).unwrap();
    let groups: Vec<_> = s
        .history
        .undo
        .iter()
        .map(|e| (e.group, e.commands.len()))
        .collect();
    assert_eq!(
        groups,
        [(None, 1), (turn.group, 1), (None, 1), (turn.group, 1)],
        "the fixture's rename, the turn, the user's edit, the turn again"
    );
    // Undone in the reverse order of application, one entry at a time.
    run(&h(history::Command::Undo {}), &mut s, &mut context()).unwrap();
    assert_eq!(s.project.name, "Demo");
    assert_eq!(param(&s, &f.device, "gain"), 0.2);
    run(&h(history::Command::Undo {}), &mut s, &mut context()).unwrap();
    run(&h(history::Command::Undo {}), &mut s, &mut context()).unwrap();
    assert_eq!(s.project, f.state.project);
}

#[test]
fn a_group_its_author_abandons_is_closed_and_undo_works_again() {
    let f = fixture();
    let mut s = f.state.clone();
    run(
        &h(history::Command::BeginGroup {}),
        &mut s,
        &mut agent("claude"),
    )
    .unwrap();
    run(&set(&f.device, "attack", 0.1), &mut s, &mut agent("claude")).unwrap();
    // The agent goes away without ending its group: undo is refused to the user...
    check_refusal(&h(history::Command::Undo {}), &s, &context()).unwrap();
    // ...until the host abandons the group for that author, and only for that author.
    assert!(!s.history.abandon_group(&Author::User));
    assert!(s.history.open_group.is_some());
    assert!(s.history.abandon_group(&Author::Agent {
        name: "claude".into()
    }));
    assert_eq!(s.history.open_group, None);
    assert!(!s.history.abandon_group(&Author::Agent {
        name: "claude".into()
    }));
    run(&h(history::Command::Undo {}), &mut s, &mut context()).unwrap();
    assert_eq!(
        s.project, f.state.project,
        "the abandoned group's edits are one entry"
    );
}

#[test]
fn a_redo_that_fails_halfway_leaves_the_entry_on_the_redo_stack() {
    let f = fixture();
    let mut s = f.state.clone();
    let missing = ::project::DeviceId::parse("01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap();
    s.history.redo.push(Entry {
        id: 0,
        at: 0,
        name: "device.set_param",
        subject: None,
        others: 0,
        author: Author::User,
        group: Some(GroupId(99)),
        commands: vec![set(&missing, "attack", 0.1), rename("Half")],
    });
    let err = check_refusal(&h(history::Command::Redo {}), &s, &context()).unwrap();
    assert_eq!(err.code(), "not_found");
    let mut after = s.clone();
    assert!(run(&h(history::Command::Redo {}), &mut after, &mut context()).is_err());
    assert_eq!(
        after, s,
        "the entry is back on the redo stack, the undo stack untouched"
    );
}

#[test]
fn a_group_join_keeps_the_entry_id_and_time() {
    let f = fixture();
    let mut s = f.state.clone();
    run(&h(history::Command::BeginGroup {}), &mut s, &mut context()).unwrap();
    let mut at_first = context();
    at_first.now_ms = 1_000;
    run(&set(&f.device, "attack", 0.1), &mut s, &mut at_first).unwrap();
    let (id, at) = {
        let top = s.history.undo.last().unwrap();
        (top.id, top.at)
    };
    assert_eq!(at, 1_000);
    let mut later = context();
    later.now_ms = 9_000;
    run(&set(&f.device, "attack", 0.2), &mut s, &mut later).unwrap();
    let top = s.history.undo.last().unwrap();
    assert_eq!(top.commands.len(), 2, "the second edit joined the entry");
    assert_eq!(
        (top.id, top.at),
        (id, at),
        "a join changes neither id nor time"
    );
    assert_eq!(s.history.view().undo.last().unwrap().id, id);
}
