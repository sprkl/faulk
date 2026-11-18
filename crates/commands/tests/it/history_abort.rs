//! `history.abort_group` (#867): a group taken back as if it had never been opened.

use commands::testing::{check_refusal, context, fixture};
use commands::{
    Author, Command, Context, Kind, Meta, ProjectEvent, State, Undo, UndoKind, device, history, run,
};

fn h(c: history::Command) -> Command {
    Command::History(c)
}

fn set(device: &::project::DeviceId, param: &str, value: f64) -> Command {
    Command::Device(device::Command::SetParam {
        device: device.clone(),
        param: param.into(),
        value,
    })
}

fn agent(name: &str) -> Context {
    let mut c = context();
    c.author = Author::Agent { name: name.into() };
    c
}

fn undone(s: &State, out: &commands::Output) -> u32 {
    let _ = s;
    let commands::Output::History(history::Output::AbortGroup { undone, .. }) = out else {
        panic!("{out:?}")
    };
    *undone
}

#[test]
fn aborting_a_group_takes_back_its_edits_and_leaves_no_entry() {
    let f = fixture();
    let mut s = f.state.clone();
    let before = s.project.clone();
    let entries = s.history.undo.clone();
    run(&h(history::Command::BeginGroup {}), &mut s, &mut context()).unwrap();
    for v in [0.1, 0.2] {
        run(&set(&f.device, "attack", v), &mut s, &mut context()).unwrap();
    }
    run(&set(&f.device, "gain", 0.5), &mut s, &mut context()).unwrap();
    assert_ne!(s.project, before);
    let out = run(&h(history::Command::AbortGroup {}), &mut s, &mut context()).unwrap();
    assert_eq!(undone(&s, &out.output), 3);
    assert_eq!(s.project, before, "the project is as before the group");
    assert_eq!(s.history.undo, entries, "no entry is left");
    assert_eq!(s.history.open_group, None);
    assert!(
        out.events
            .iter()
            .any(|e| matches!(e, ProjectEvent::HistoryChanged { .. }))
    );
    // The history works again.
    run(&h(history::Command::Undo {}), &mut s, &mut context()).unwrap();
}

#[test]
fn aborting_a_group_puts_back_what_could_be_redone_before_it() {
    let f = fixture();
    let mut s = f.state.clone();
    run(&h(history::Command::Undo {}), &mut s, &mut context()).unwrap();
    let redo = s.history.redo.clone();
    assert_eq!(redo.len(), 1);
    run(&h(history::Command::BeginGroup {}), &mut s, &mut context()).unwrap();
    run(&set(&f.device, "attack", 0.1), &mut s, &mut context()).unwrap();
    assert!(s.history.redo.is_empty(), "an edit empties the redo stack");
    run(&h(history::Command::AbortGroup {}), &mut s, &mut context()).unwrap();
    assert_eq!(s.history.redo, redo);
    run(&h(history::Command::Redo {}), &mut s, &mut context()).unwrap();
    // A group that ends normally does not give them back.
    run(&h(history::Command::Undo {}), &mut s, &mut context()).unwrap();
    run(&h(history::Command::BeginGroup {}), &mut s, &mut context()).unwrap();
    run(&set(&f.device, "attack", 0.1), &mut s, &mut context()).unwrap();
    run(&h(history::Command::EndGroup {}), &mut s, &mut context()).unwrap();
    assert!(s.history.redo.is_empty());
    assert!(s.history.group_redo.is_empty());
}

#[test]
fn aborting_an_empty_group_only_closes_it() {
    let f = fixture();
    let mut s = f.state.clone();
    run(&h(history::Command::Undo {}), &mut s, &mut context()).unwrap();
    run(&h(history::Command::BeginGroup {}), &mut s, &mut context()).unwrap();
    let before = s.clone();
    let out = run(&h(history::Command::AbortGroup {}), &mut s, &mut context()).unwrap();
    assert_eq!(undone(&s, &out.output), 0);
    assert!(out.events.is_empty());
    assert_eq!(s.history.open_group, None);
    assert_eq!(s.project, before.project);
    assert_eq!(s.history.undo, before.history.undo);
    assert_eq!(s.history.redo, before.history.redo);
}

#[test]
fn aborting_with_no_group_or_another_authors_group_is_refused() {
    let f = fixture();
    let err = check_refusal(&h(history::Command::AbortGroup {}), &f.state, &context()).unwrap();
    assert_eq!(err.code(), "invalid");
    let mut s = f.state.clone();
    run(
        &h(history::Command::BeginGroup {}),
        &mut s,
        &mut agent("claude"),
    )
    .unwrap();
    run(&set(&f.device, "attack", 0.1), &mut s, &mut agent("claude")).unwrap();
    let err = check_refusal(&h(history::Command::AbortGroup {}), &s, &context()).unwrap();
    assert!(err.to_string().contains("agent:claude"), "{err}");
}

#[test]
fn aborting_a_group_another_edit_landed_inside_is_refused_and_changes_nothing() {
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
    run(&set(&f.device, "attack", 0.3), &mut s, &mut agent("claude")).unwrap();
    let after = s.clone();
    let err = run(
        &h(history::Command::AbortGroup {}),
        &mut s,
        &mut agent("claude"),
    )
    .unwrap_err();
    assert!(err.to_string().contains("landed inside"), "{err}");
    assert_eq!(s, after, "nothing moved, the group is still open");
}

#[test]
fn an_agent_turn_does_not_continue_over_an_aborted_group() {
    let f = fixture();
    let mut s = f.state.clone();
    run(
        &h(history::Command::BeginGroup {}),
        &mut s,
        &mut agent("claude"),
    )
    .unwrap();
    run(&set(&f.device, "attack", 0.1), &mut s, &mut agent("claude")).unwrap();
    run(
        &h(history::Command::AbortGroup {}),
        &mut s,
        &mut agent("claude"),
    )
    .unwrap();
    assert_eq!(s.history.turn, None);
    let entries = s.history.undo.len();
    run(&set(&f.device, "attack", 0.4), &mut s, &mut agent("claude")).unwrap();
    assert_eq!(s.history.undo.len(), entries + 1);
}

#[test]
fn aborting_a_group_an_irreversible_edit_emptied_the_history_inside_is_refused() {
    let f = fixture();
    let mut s = f.state.clone();
    // Even on a history that was empty when the group began.
    s.history.undo.clear();
    run(&h(history::Command::BeginGroup {}), &mut s, &mut context()).unwrap();
    run(&set(&f.device, "attack", 0.1), &mut s, &mut context()).unwrap();
    // What an irreversible edit does to the history (`track.delete`, `project.new`): both stacks emptied.
    let destructive = Meta {
        name: "track.delete",
        kind: Kind::Edit,
        undo: UndoKind::None,
        destructive: true,
    };
    s.history
        .record(destructive, &Undo::None, &Author::User, None, 0, None);
    run(&set(&f.device, "gain", 0.2), &mut s, &mut context()).unwrap();
    let after = s.clone();
    let err = run(&h(history::Command::AbortGroup {}), &mut s, &mut context()).unwrap_err();
    assert!(err.to_string().contains("irreversible"), "{err}");
    assert_eq!(s, after, "nothing moved, the group is still open");
}
