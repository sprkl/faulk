//! `history.*`: undo, redo and history groups (PR-2), and [`run`], the hosts' entry
//! point that records every undoable edit in the [`History`].
//!
//! Undo applies the recorded inverses through [`crate::Command::apply`], the path of
//! every command, so the interface and the engine follow an undo like any edit
//! (contract, `contract.rs`). An entry gathers the inverses of one command, or of every
//! command of one history group (a mouse drag, an agent turn): undoing it undoes them
//! all, in reverse order.

use crate::contract::{
    Author, CommandError, Context, ControlMsg, GroupId, Kind, Meta, Outcome, ProjectEvent, State,
    Undo,
};
use crate::macros::domain;

domain! {
    domain = "history", schema = ("HistoryCommand", "HistoryOutput");

    /// Undo the last entry of the history: one command, or every command of one
    /// history group. Does nothing when there is nothing to undo; refused while a
    /// group is open. Undo: `history.redo`.
    "history.undo" (kind = "edit", undo = "inverse", destructive = false)
    Undo {} => {
        /// The name of the first command of the entry undone; `null` when there was
        /// nothing to undo.
        name: Option<String>,
        /// Who ran the entry undone.
        author: Option<Author>,
    };

    /// Redo the last entry undone. Does nothing when there is nothing to redo; refused
    /// while a group is open. A new edit empties what can be redone. Undo:
    /// `history.undo`.
    "history.redo" (kind = "edit", undo = "inverse", destructive = false)
    Redo {} => {
        /// The name of the first command of the entry redone; `null` when there was
        /// nothing to redo.
        name: Option<String>,
        /// Who ran the entry redone.
        author: Option<Author>,
    };

    /// Open a history group: every edit of the same author until `history.end_group`
    /// becomes one entry, undone at once (a mouse drag). Refused when a group is
    /// already open (groups do not nest). Undo and redo are refused while it is open:
    /// the host ends it when the gesture or the turn ends. Not undoable itself.
    "history.begin_group" (kind = "live", undo = "none", destructive = false)
    BeginGroup {} => {
        /// The group opened.
        group: GroupId,
    };

    /// Close the open history group. Refused when no group is open, or when another
    /// author opened it. Not undoable itself.
    "history.end_group" (kind = "live", undo = "none", destructive = false)
    EndGroup {} => {
        /// The group closed.
        group: GroupId,
    };
}

/// One entry of the history: the inverses of one command or of one group.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    /// Identifies the entry for the history panel: unique within the [`History`], kept when
    /// the entry moves between the undo and the redo stack and when a later edit of its
    /// group joins it. The history panel keys its rows by it.
    pub id: u64,
    /// When the entry was recorded, in the host's milliseconds (`Context::now_ms`); an undo
    /// and a redo keep it, a later edit of the group that joins it does not change it.
    pub at: u64,
    /// The name of the entry's first command, for the history panel (PR-7).
    pub name: &'static str,
    pub author: Author,
    pub group: Option<GroupId>,
    /// What undoes (on the undo stack) or redoes (on the redo stack) the entry, in
    /// the order the commands were applied; applied in reverse order.
    pub commands: Vec<crate::Command>,
}

/// The undo and redo stacks, and the open group. Kept in the [`State`]; changed only by
/// [`run`] and the `history.*` commands.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct History {
    pub undo: Vec<Entry>,
    pub redo: Vec<Entry>,
    /// The group opened by `history.begin_group`, if any, and who opened it: only that
    /// author's edits join it, and only that author ends it.
    pub open_group: Option<(GroupId, Author)>,
    /// The number of the next group this history gives out.
    pub next_group: u64,
    /// The id of the next entry this history records.
    pub next_entry: u64,
}

impl History {
    /// A new group id, unique within this history: what `history.begin_group` opens,
    /// and what a host gives an agent turn as its [`Context::group`].
    pub fn new_group(&mut self) -> GroupId {
        self.next_group += 1;
        GroupId(self.next_group)
    }

    /// Closes the group `author` left open, if any: what a host calls when that author
    /// goes away without `history.end_group` (an agent's connection drops), since an
    /// open group refuses undo and redo to everyone and only its author ends it.
    /// Returns whether a group was closed.
    pub fn abandon_group(&mut self, author: &Author) -> bool {
        match &self.open_group {
            Some((_, opener)) if opener == author => {
                self.open_group = None;
                true
            }
            _ => false,
        }
    }

    /// The entries as the history panel shows them (oldest first).
    pub fn view(&self) -> crate::host::HistoryView {
        let rows = |entries: &[Entry]| {
            entries
                .iter()
                .map(|entry| crate::host::HistoryEntryView {
                    id: entry.id,
                    at: entry.at,
                    name: entry.name.to_owned(),
                    author: entry.author.clone(),
                    group: entry.group,
                })
                .collect()
        };
        crate::host::HistoryView {
            undo: rows(&self.undo),
            redo: rows(&self.redo),
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// The event that tells the interface the history changed.
    pub fn changed_event(&self) -> ProjectEvent {
        ProjectEvent::HistoryChanged {
            can_undo: self.can_undo(),
            can_redo: self.can_redo(),
        }
    }

    /// Records what an applied command left to undo. Returns whether the history
    /// changed.
    ///
    /// - An edit with an inverse becomes a new entry, or joins the last entry when both
    ///   belong to the same group; what could be redone is dropped. Only the top entry
    ///   is joined: when another author's edit lands between two edits of a group, the
    ///   group goes on in a new entry, so the history stays in the order the edits were
    ///   applied (joining an older entry would undo it before a later edit it may
    ///   depend on). One undo then undoes the group's edits made after that edit.
    /// - An irreversible edit (destructive, no inverse) empties both stacks: the
    ///   inverses recorded before it may no longer apply.
    /// - Anything else (an edit that changed nothing, `live`, `query`) leaves the
    ///   history as it is.
    pub fn record(
        &mut self,
        meta: Meta,
        undo: &Undo<crate::Command>,
        author: &Author,
        group: Option<GroupId>,
        at: u64,
    ) -> bool {
        match (meta.kind, undo) {
            (Kind::Edit, Undo::Inverse(inverse)) => {
                self.redo.clear();
                match self.undo.last_mut() {
                    Some(top) if group.is_some() && top.group == group => {
                        top.commands.push((**inverse).clone());
                    }
                    _ => {
                        self.next_entry += 1;
                        self.undo.push(Entry {
                            id: self.next_entry,
                            at,
                            name: meta.name,
                            author: author.clone(),
                            group,
                            commands: vec![(**inverse).clone()],
                        });
                    }
                }
                true
            }
            (Kind::Edit, Undo::None) if meta.destructive => {
                let changed = self.can_undo() || self.can_redo();
                self.undo.clear();
                self.redo.clear();
                changed
            }
            _ => false,
        }
    }
}

/// Runs a command and records it in the history: what a host calls for every command
/// (the page, the MCP server, an agent turn). The history group is the context's, or
/// else the group opened by `history.begin_group`. The `history.*` commands themselves
/// are never recorded. A refused command changes nothing, the history included.
pub fn run(
    command: &crate::Command,
    state: &mut State,
    ctx: &mut Context,
) -> Result<crate::Outcome, CommandError> {
    let mut outcome = command.apply(state, ctx)?;
    if matches!(command, crate::Command::History(_)) {
        return Ok(outcome);
    }
    let group = ctx.group.or_else(|| {
        state
            .history
            .open_group
            .as_ref()
            .filter(|(_, opener)| *opener == ctx.author)
            .map(|(group, _)| *group)
    });
    if state.history.record(
        command.meta(),
        &outcome.undo,
        &ctx.author,
        group,
        ctx.now_ms,
    ) {
        outcome.events.push(state.history.changed_event());
    }
    Ok(outcome)
}

/// Which stack an entry is taken from.
#[derive(Clone, Copy)]
enum Direction {
    Undo,
    Redo,
}

/// Applies a command of this domain.
pub fn apply(
    command: &Command,
    state: &mut State,
    ctx: &mut Context,
) -> Result<Outcome<Command, Output>, CommandError> {
    match command {
        Command::Undo {} => step(Direction::Undo, state, ctx),
        Command::Redo {} => step(Direction::Redo, state, ctx),
        Command::BeginGroup {} => {
            if let Some((open, _)) = &state.history.open_group {
                return Err(CommandError::Invalid {
                    field: "group".into(),
                    message: format!("group {} is already open; groups do not nest", open.0),
                });
            }
            let group = state.history.new_group();
            state.history.open_group = Some((group, ctx.author.clone()));
            Ok(live(Output::BeginGroup { group }))
        }
        Command::EndGroup {} => match &state.history.open_group {
            Some((group, opener)) if *opener == ctx.author => {
                let group = *group;
                state.history.open_group = None;
                Ok(live(Output::EndGroup { group }))
            }
            Some((group, opener)) => Err(CommandError::Invalid {
                field: "group".into(),
                message: format!(
                    "group {} was opened by {}; only its author ends it",
                    group.0,
                    String::from(opener.clone())
                ),
            }),
            None => Err(CommandError::Invalid {
                field: "group".into(),
                message: "no history group is open".into(),
            }),
        },
    }
}

fn live(output: Output) -> Outcome<Command, Output> {
    Outcome {
        output,
        undo: Undo::None,
        events: vec![],
        control: vec![],
    }
}

/// Builds the output of `history.undo` or `history.redo` from the entry it applied.
type MakeOutput = fn(Option<String>, Option<Author>) -> Output;

/// Undoes or redoes the top entry of one stack and pushes what reverses it on the other.
fn step(
    direction: Direction,
    state: &mut State,
    ctx: &mut Context,
) -> Result<Outcome<Command, Output>, CommandError> {
    if let Some((open, _)) = &state.history.open_group {
        return Err(CommandError::Invalid {
            field: "group".into(),
            message: format!("group {} is open; end it first", open.0),
        });
    }
    let (from, output, inverse): (_, MakeOutput, _) = match direction {
        Direction::Undo => (
            &mut state.history.undo,
            (|name, author| Output::Undo { name, author }) as _,
            Command::Redo {},
        ),
        Direction::Redo => (
            &mut state.history.redo,
            (|name, author| Output::Redo { name, author }) as _,
            Command::Undo {},
        ),
    };
    let Some(entry) = from.pop() else {
        // Nothing to undo or redo: nothing changes.
        return Ok(live(output(None, None)));
    };
    let applied = match apply_entry(&entry, state, ctx) {
        Ok(applied) => applied,
        Err(error) => {
            // The state is as it was: put the entry back so the history is too.
            match direction {
                Direction::Undo => state.history.undo.push(entry),
                Direction::Redo => state.history.redo.push(entry),
            }
            return Err(error);
        }
    };
    let (reverse, mut events, control) = applied;
    let out = output(Some(entry.name.to_owned()), Some(entry.author.clone()));
    let reversed = Entry {
        commands: reverse,
        ..entry
    };
    match direction {
        Direction::Undo => state.history.redo.push(reversed),
        Direction::Redo => state.history.undo.push(reversed),
    }
    events.push(state.history.changed_event());
    Ok(Outcome {
        output: out,
        undo: Undo::Inverse(Box::new(inverse)),
        events,
        control,
    })
}

/// What applying an entry gives: the commands that reverse it, in application order,
/// and the events and engine messages of its commands.
type Applied = (Vec<crate::Command>, Vec<ProjectEvent>, Vec<ControlMsg>);

/// Applies an entry's commands in reverse order. All or nothing: a command that fails
/// leaves the project, the session and the engine's slot table as they were before the
/// entry (each command is atomic on its own; an entry of several commands is applied on a
/// saved copy).
fn apply_entry(
    entry: &Entry,
    state: &mut State,
    ctx: &mut Context,
) -> Result<Applied, CommandError> {
    // The engine's slot table too: the messages of the commands already applied are
    // dropped with the error, so the engine never saw them.
    let saved = (entry.commands.len() > 1).then(|| {
        (
            state.project.clone(),
            state.session.clone(),
            ctx.engine.clone(),
        )
    });
    let mut reverse = Vec::with_capacity(entry.commands.len());
    let mut events = Vec::new();
    let mut control = Vec::new();
    for command in entry.commands.iter().rev() {
        match command.apply(state, ctx) {
            Ok(outcome) => {
                if let Undo::Inverse(inverse) = outcome.undo {
                    reverse.push(*inverse);
                }
                events.extend(outcome.events);
                control.extend(outcome.control);
            }
            Err(error) => {
                if let Some((project, session, engine)) = saved {
                    state.project = project;
                    state.session = session;
                    ctx.engine = engine;
                }
                return Err(error);
            }
        }
    }
    Ok((reverse, events, control))
}
