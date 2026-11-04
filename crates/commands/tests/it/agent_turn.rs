//! An agent's turn (AG-3): the history gives an agent's consecutive edits one group, so one
//! undo undoes the whole turn, and a user's gesture during the turn is never swallowed into
//! it. One test per transition of the turn (continue, gap, another author, undo, redo,
//! irreversible edit, no-op, refusal, explicit groups, two agents).

use commands::history::TURN_GAP_MS;
use commands::testing::{check_refusal, context, fixture};
use commands::{
    Author, Command, Context, GroupId, Kind, Meta, State, Undo, UndoKind, device, history, project,
    run, track,
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

/// An agent's context at `now_ms`.
fn agent(name: &str, now_ms: u64) -> Context {
    let mut c = context();
    c.author = Author::Agent { name: name.into() };
    c.now_ms = now_ms;
    c
}

/// The user's context at `now_ms`.
fn user(now_ms: u64) -> Context {
    let mut c = context();
    c.now_ms = now_ms;
    c
}

/// The undo stack as (author, group, number of commands), oldest first.
fn stack(s: &State) -> Vec<(String, Option<GroupId>, usize)> {
    s.history
        .undo
        .iter()
        .map(|e| (String::from(e.author.clone()), e.group, e.commands.len()))
        .collect()
}

fn undo(s: &mut State) {
    run(&h(history::Command::Undo {}), s, &mut user(0)).unwrap();
}

#[test]
fn an_agents_consecutive_edits_are_one_entry_and_one_undo_undoes_the_turn() {
    let f = fixture();
    let mut s = f.state.clone();
    let before = s.project.clone();
    run(
        &set(&f.device, "attack", 0.1),
        &mut s,
        &mut agent("claude", 1_000),
    )
    .unwrap();
    run(&rename("By agent"), &mut s, &mut agent("claude", 5_000)).unwrap();
    run(
        &set(&f.device, "gain", 0.2),
        &mut s,
        &mut agent("claude", 20_000),
    )
    .unwrap();
    let top = s.history.undo.last().unwrap();
    assert_eq!(
        s.history.undo.len(),
        2,
        "the fixture's rename, then the turn"
    );
    assert_eq!(top.commands.len(), 3);
    assert!(top.group.is_some());
    assert_eq!(
        top.at, 1_000,
        "the turn's entry keeps the time of its first edit"
    );
    assert_eq!(s.history.view().undo.last().unwrap().edits, 3);
    let edited = s.project.clone();
    undo(&mut s);
    assert_eq!(s.project, before, "one undo undoes the whole turn");
    run(&h(history::Command::Redo {}), &mut s, &mut user(0)).unwrap();
    assert_eq!(s.project, edited, "one redo brings the whole turn back");
}

#[test]
fn a_users_gesture_during_a_turn_is_not_swallowed_and_ends_the_turn() {
    let f = fixture();
    let mut s = f.state.clone();
    let before = s.project.clone();
    run(
        &set(&f.device, "attack", 0.1),
        &mut s,
        &mut agent("claude", 0),
    )
    .unwrap();
    run(
        &set(&f.device, "attack", 0.2),
        &mut s,
        &mut agent("claude", 1),
    )
    .unwrap();
    // The user's gesture lands in the middle of the turn.
    run(&rename("Mine"), &mut s, &mut user(2)).unwrap();
    run(
        &set(&f.device, "gain", 0.3),
        &mut s,
        &mut agent("claude", 3),
    )
    .unwrap();
    let rows = stack(&s);
    assert_eq!(rows.len(), 4);
    assert_eq!(
        &rows[2],
        &("user".to_owned(), None, 1),
        "the gesture has its own entry"
    );
    let (first, second) = (rows[1].1.unwrap(), rows[3].1.unwrap());
    assert_ne!(
        first, second,
        "the agent's edits after the gesture are a new turn"
    );
    assert_eq!((rows[1].2, rows[3].2), (2, 1));
    // Undone in the order the edits were applied: never the gesture with the turn.
    undo(&mut s);
    assert_eq!(s.project.name, "Mine", "the gesture is still there");
    assert_eq!(super_param(&s, &f.device, "attack"), 0.2);
    undo(&mut s);
    assert_eq!(s.project.name, before.name);
    assert_eq!(super_param(&s, &f.device, "attack"), 0.2);
    undo(&mut s);
    assert_eq!(s.project, before);
}

#[test]
fn a_pause_longer_than_the_gap_starts_a_new_turn() {
    let f = fixture();
    let mut s = f.state.clone();
    run(
        &set(&f.device, "attack", 0.1),
        &mut s,
        &mut agent("claude", 1_000),
    )
    .unwrap();
    // Exactly the gap: still the same turn.
    run(
        &set(&f.device, "attack", 0.2),
        &mut s,
        &mut agent("claude", 1_000 + TURN_GAP_MS),
    )
    .unwrap();
    assert_eq!(s.history.undo.len(), 2);
    // One millisecond more after that edit: a new turn.
    run(
        &set(&f.device, "attack", 0.3),
        &mut s,
        &mut agent("claude", 1_000 + 2 * TURN_GAP_MS + 1),
    )
    .unwrap();
    let rows = stack(&s);
    assert_eq!(rows.len(), 3);
    assert_ne!(rows[1].1, rows[2].1);
    assert_eq!((rows[1].2, rows[2].2), (2, 1));
    // A clock that goes back keeps the turn going (no gap measured).
    run(
        &set(&f.device, "attack", 0.4),
        &mut s,
        &mut agent("claude", 0),
    )
    .unwrap();
    assert_eq!(s.history.undo.len(), 3);
}

#[test]
fn an_undo_or_a_redo_during_a_turn_ends_it() {
    let f = fixture();
    let mut s = f.state.clone();
    run(
        &set(&f.device, "attack", 0.1),
        &mut s,
        &mut agent("claude", 0),
    )
    .unwrap();
    run(
        &set(&f.device, "attack", 0.2),
        &mut s,
        &mut agent("claude", 1),
    )
    .unwrap();
    // The user undoes the turn and redoes it: the same entry is on top again...
    undo(&mut s);
    run(&h(history::Command::Redo {}), &mut s, &mut user(2)).unwrap();
    let turn = s.history.undo.last().unwrap().group;
    // ...but the agent's next edit is a new turn: it does not join what the user undid.
    run(
        &set(&f.device, "gain", 0.3),
        &mut s,
        &mut agent("claude", 3),
    )
    .unwrap();
    let rows = stack(&s);
    assert_eq!(rows.len(), 3);
    assert_ne!(rows[2].1, turn);
    // The agent's own undo ends its turn too.
    run(
        &h(history::Command::Undo {}),
        &mut s,
        &mut agent("claude", 4),
    )
    .unwrap();
    run(
        &set(&f.device, "gain", 0.4),
        &mut s,
        &mut agent("claude", 5),
    )
    .unwrap();
    run(
        &set(&f.device, "gain", 0.5),
        &mut s,
        &mut agent("claude", 6),
    )
    .unwrap();
    let rows = stack(&s);
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[2].2, 2, "the edits after the undo are one new turn");
}

#[test]
fn another_agents_edit_ends_the_turn() {
    let f = fixture();
    let mut s = f.state.clone();
    run(
        &set(&f.device, "attack", 0.1),
        &mut s,
        &mut agent("claude", 0),
    )
    .unwrap();
    run(
        &set(&f.device, "attack", 0.2),
        &mut s,
        &mut agent("other", 1),
    )
    .unwrap();
    run(
        &set(&f.device, "attack", 0.3),
        &mut s,
        &mut agent("claude", 2),
    )
    .unwrap();
    let rows = stack(&s);
    assert_eq!(
        rows[1..]
            .iter()
            .map(|(a, _, n)| (a.as_str(), *n))
            .collect::<Vec<_>>(),
        [("agent:claude", 1), ("agent:other", 1), ("agent:claude", 1)]
    );
    let groups: Vec<_> = rows[1..].iter().map(|r| r.1.unwrap()).collect();
    assert!(groups[0] != groups[1] && groups[1] != groups[2] && groups[0] != groups[2]);
}

#[test]
fn an_irreversible_edit_ends_the_turn() {
    let f = fixture();
    let mut s = f.state.clone();
    run(
        &set(&f.device, "attack", 0.1),
        &mut s,
        &mut agent("claude", 0),
    )
    .unwrap();
    // An irreversible edit empties both stacks: no turn entry is left to join.
    let destructive = Meta {
        name: "track.delete",
        kind: Kind::Edit,
        undo: UndoKind::None,
        destructive: true,
    };
    assert!(
        s.history
            .record(destructive, &Undo::None, &Author::User, None, 0, None)
    );
    run(
        &set(&f.device, "attack", 0.2),
        &mut s,
        &mut agent("claude", 1),
    )
    .unwrap();
    assert_eq!(s.history.undo.len(), 1);
    assert_eq!(s.history.undo[0].edits, 1);
}

#[test]
fn the_users_edits_are_never_a_turn() {
    let f = fixture();
    let mut s = f.state.clone();
    run(&set(&f.device, "attack", 0.1), &mut s, &mut user(0)).unwrap();
    run(&set(&f.device, "attack", 0.2), &mut s, &mut user(1)).unwrap();
    let rows = stack(&s);
    assert_eq!(rows.len(), 3);
    assert_eq!((rows[1].1, rows[2].1), (None, None));
    assert_eq!(s.history.turn, None);
}

#[test]
fn what_records_nothing_leaves_the_turn_alone() {
    let f = fixture();
    let mut s = f.state.clone();
    run(
        &set(&f.device, "attack", 0.1),
        &mut s,
        &mut agent("claude", 0),
    )
    .unwrap();
    let history = s.history.clone();
    // An edit that changes nothing, a live command, a query, by the agent or the user.
    for mut ctx in [agent("claude", 1), user(1)] {
        run(&set(&f.device, "attack", 0.1), &mut s, &mut ctx).unwrap();
        run(
            &Command::Live(commands::live::Command::AllNotesOff {}),
            &mut s,
            &mut ctx,
        )
        .unwrap();
        run(
            &Command::Track(track::Command::List {
                offset: None,
                limit: None,
            }),
            &mut s,
            &mut ctx,
        )
        .unwrap();
        assert_eq!(s.history, history);
    }
    // A refused command changes nothing, the turn included.
    let missing = ::project::DeviceId::parse("01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap();
    check_refusal(&set(&missing, "attack", 0.5), &s, &agent("claude", 2)).unwrap();
    // The turn goes on.
    run(
        &set(&f.device, "attack", 0.2),
        &mut s,
        &mut agent("claude", 3),
    )
    .unwrap();
    assert_eq!(s.history.undo.last().unwrap().commands.len(), 2);
}

#[test]
fn an_agents_own_group_and_the_hosts_group_win_over_the_turn() {
    let f = fixture();
    let mut s = f.state.clone();
    // The agent bounds its turn itself.
    run(
        &h(history::Command::BeginGroup {}),
        &mut s,
        &mut agent("claude", 0),
    )
    .unwrap();
    let opened = s.history.open_group.clone().unwrap().0;
    run(
        &set(&f.device, "attack", 0.1),
        &mut s,
        &mut agent("claude", 1),
    )
    .unwrap();
    run(
        &set(&f.device, "attack", 0.2),
        &mut s,
        &mut agent("claude", 2 * TURN_GAP_MS),
    )
    .unwrap();
    run(
        &h(history::Command::EndGroup {}),
        &mut s,
        &mut agent("claude", 3),
    )
    .unwrap();
    assert_eq!(s.history.undo.last().unwrap().group, Some(opened));
    assert_eq!(
        s.history.undo.last().unwrap().commands.len(),
        2,
        "no gap inside its own group"
    );
    // A group begun in the middle of a turn: its edits are its own entry, and the turn
    // does not join it afterwards.
    run(
        &set(&f.device, "attack", 0.3),
        &mut s,
        &mut agent("claude", 4),
    )
    .unwrap();
    run(
        &h(history::Command::BeginGroup {}),
        &mut s,
        &mut agent("claude", 4),
    )
    .unwrap();
    run(
        &set(&f.device, "attack", 0.4),
        &mut s,
        &mut agent("claude", 4),
    )
    .unwrap();
    run(
        &h(history::Command::EndGroup {}),
        &mut s,
        &mut agent("claude", 4),
    )
    .unwrap();
    run(
        &set(&f.device, "attack", 0.5),
        &mut s,
        &mut agent("claude", 4),
    )
    .unwrap();
    let edits: Vec<_> = s
        .history
        .undo
        .iter()
        .rev()
        .take(3)
        .map(|e| e.edits)
        .collect();
    assert_eq!(edits, [1, 1, 1], "the turn, the group, a new turn");
    // A host's group wins over everything.
    let mut hosted = agent("claude", 4);
    hosted.group = Some(s.history.new_group());
    run(&set(&f.device, "gain", 0.1), &mut s, &mut hosted).unwrap();
    assert_eq!(s.history.undo.last().unwrap().group, hosted.group);
    // After them, the agent's edit opens a turn of its own.
    run(
        &set(&f.device, "gain", 0.2),
        &mut s,
        &mut agent("claude", 5),
    )
    .unwrap();
    let top = s.history.undo.last().unwrap();
    assert_ne!(top.group, hosted.group);
    assert_eq!(top.commands.len(), 1);
}

fn super_param(s: &State, d: &::project::DeviceId, p: &str) -> f64 {
    s.project
        .tracks
        .values()
        .find_map(|t| t.devices.get(d))
        .unwrap()
        .params[p]
}
