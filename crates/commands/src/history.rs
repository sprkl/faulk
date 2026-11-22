//! `history.*`: undo, redo and history groups (PR-2), and [`run`], the hosts' entry
//! point that records every undoable edit in the [`History`].
//!
//! Undo applies the recorded inverses through [`crate::Command::apply`], the path of
//! every command, so the interface and the engine follow an undo like any edit
//! (contract, `contract.rs`). An entry gathers the inverses of one command, or of every
//! command of one history group (a mouse drag, an agent turn): undoing it undoes them
//! all, in reverse order.
//!
//! An agent's turn (AG-3) is a group the history gives on its own: MCP carries no turn
//! boundary, so [`run`] puts an agent's consecutive edits in one group ([`History::turn`])
//! until anything else reaches the undo history. The rule:
//!
//! - an agent's undoable edit continues its current turn when (1) the turn is that agent's,
//!   (2) the top of the undo stack is still the turn's entry and (3) at most
//!   [`TURN_GAP_MS`] passed since the turn's last edit; otherwise it starts a new turn;
//! - a user's gesture during a turn is never swallowed into it: the user's edit is an entry
//!   of its own (the user's edits are never in a turn), and since it is now on top, the
//!   agent's next edit starts a new turn. The history stays in the order the edits were
//!   applied, so one undo undoes the agent's edits after the gesture, the next one the
//!   gesture, the next one the agent's edits before it. Another agent's edit, an undo or a
//!   redo (by anyone) and an irreversible edit (which empties the stacks) end the turn too;
//! - what records nothing (an edit that changed nothing, a refusal, `live`, `query`, a
//!   `setting`, which only the activity log lists) leaves the turn as it is;
//! - a host's [`Context::group`] wins, then the group its author opened with
//!   `history.begin_group` (an agent that bounds its turn itself), then the turn.

use crate::contract::{
    Author, CommandError, Context, ControlMsg, GroupId, Kind, Meta, Outcome, ProjectEvent, State,
    Undo,
};
use crate::describe::Subject;
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

    /// Close the open history group and take back everything it recorded, as if it had
    /// never been opened: its edits are undone and leave no entry, on neither stack (a
    /// batch of edits where one was refused, #867). Refused when no group is open, when
    /// another author opened it, or when another author's edit landed inside it (the
    /// group is then no longer on top of the history). A refusal changes nothing and
    /// leaves the group open. What could be redone when the group began can be redone
    /// again. Not undoable itself.
    "history.abort_group" (kind = "live", undo = "none", destructive = false)
    AbortGroup {} => {
        /// The group closed.
        group: GroupId,
        /// How many edits of the group were taken back.
        undone: u32,
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
    /// What the commands of that name acted on, by name (#473), when they say.
    pub subject: Option<Subject>,
    /// How many commands of the entry have another name than its first (a group that mixes
    /// edits, like an agent's turn): the panel says "and n other edits".
    pub others: u32,
    pub author: Author,
    pub group: Option<GroupId>,
    /// How many commands the entry holds (an agent's turn: "3 edits"), counted as they are
    /// recorded and kept by an undo and a redo.
    pub edits: u32,
    /// What undoes (on the undo stack) or redoes (on the redo stack) the entry, in
    /// the order the commands were applied; applied in reverse order.
    pub commands: Vec<crate::Command>,
}

/// A command that changed a setting of the project outside the undo history (a
/// [`Kind::Setting`]: the loop switch, the metronome), kept so that the history panel still
/// shows who did it and when (#473).
#[derive(Clone, Debug, PartialEq)]
pub struct Activity {
    /// Taken from the same counter as [`Entry::id`]: the panel orders both by it.
    pub id: u64,
    pub at: u64,
    pub name: &'static str,
    pub subject: Option<Subject>,
    pub author: Author,
}

/// The most rows the activity log keeps; the oldest goes first.
pub const MAX_ACTIVITY: usize = 200;

/// The longest pause, in milliseconds of the host's clock, between two edits of one agent's
/// turn: an edit after a longer pause starts a new turn (two requests to the agent stay two
/// undos). MCP gives no turn boundary; this is the lead's choice of 2026-11-04 (#99).
pub const TURN_GAP_MS: u64 = 30_000;

/// An agent's turn (AG-3): the group [`run`] gives that agent's consecutive edits.
#[derive(Clone, Debug, PartialEq)]
pub struct Turn {
    pub group: GroupId,
    pub author: Author,
    /// When the turn's last edit was recorded (`Context::now_ms`).
    pub last_ms: u64,
}

/// The undo and redo stacks, the open group and the activity log. Kept in the [`State`];
/// changed only by [`run`] and the `history.*` commands.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct History {
    pub undo: Vec<Entry>,
    pub redo: Vec<Entry>,
    /// The settings changed outside the undo history, oldest first, at most
    /// [`MAX_ACTIVITY`]. Undo and redo never touch it.
    pub activity: Vec<Activity>,
    /// The group opened by `history.begin_group`, if any, and who opened it: only that
    /// author's edits join it, and only that author ends it.
    pub open_group: Option<(GroupId, Author)>,
    /// What could be redone when the open group began: the group's first edit empties
    /// the redo stack, and `history.abort_group` puts it back (the group never happened).
    pub group_redo: Vec<Entry>,
    /// Whether an irreversible edit emptied the stacks since the open group began: an abort is
    /// then refused, since it could not take everything back.
    pub group_emptied: bool,
    /// The number of the next group this history gives out.
    pub next_group: u64,
    /// The agent's turn going on, if any (the rule: the module's doc). Ended by an undo or
    /// a redo; another author's edit or an irreversible edit ends it by changing the top
    /// of the undo stack.
    pub turn: Option<Turn>,
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

    /// The group of an agent's edit that [`run`] is about to record: its current turn's
    /// when the edit continues it (the module's rule), else a new turn's. Remembers the turn
    /// and the edit's time.
    pub fn turn_group(&mut self, author: &Author, now_ms: u64) -> GroupId {
        let top = self.undo.last().and_then(|entry| entry.group);
        let current = self
            .turn
            .as_ref()
            .filter(|turn| {
                turn.author == *author
                    && top == Some(turn.group)
                    // A clock that goes back measures no pause.
                    && now_ms.saturating_sub(turn.last_ms) <= TURN_GAP_MS
            })
            .map(|turn| turn.group);
        let group = current.unwrap_or_else(|| self.new_group());
        self.turn = Some(Turn {
            group,
            author: author.clone(),
            last_ms: now_ms,
        });
        group
    }

    /// Closes the group `author` left open, if any: what a host calls when that author
    /// goes away without `history.end_group` (an agent's connection drops), since an
    /// open group refuses undo and redo to everyone and only its author ends it.
    /// Returns whether a group was closed.
    pub fn abandon_group(&mut self, author: &Author) -> bool {
        match &self.open_group {
            Some((_, opener)) if opener == author => {
                self.open_group = None;
                self.group_redo.clear();
                true
            }
            _ => false,
        }
    }

    /// The agent `author` went away (#641: the helper says its last MCP session ended): closes
    /// the group it left open ([`Self::abandon_group`]) and ends its turn, so that the user can
    /// undo at once and an agent of the same name that comes back starts a new turn. Nothing
    /// for the user, nor for another agent. Returns whether anything changed.
    pub fn agent_left(&mut self, author: &Author) -> bool {
        if !matches!(author, Author::Agent { .. }) {
            return false;
        }
        let closed = self.abandon_group(author);
        let ended = self.turn.as_ref().is_some_and(|t| t.author == *author);
        if ended {
            self.turn = None;
        }
        closed || ended
    }

    /// Every agent is gone (#641: the page lost every link to the helper, so it can learn no
    /// agent's leave and no agent can reach it): closes the group an agent left open, never
    /// the user's, and ends any agent's turn. Returns whether anything changed.
    pub fn agents_gone(&mut self) -> bool {
        let closed = match &self.open_group {
            Some((_, opener @ Author::Agent { .. })) => {
                let opener = opener.clone();
                self.abandon_group(&opener)
            }
            _ => false,
        };
        let ended = self.turn.take().is_some();
        closed || ended
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
                    subject: entry.subject.clone(),
                    others: entry.others,
                    author: entry.author.clone(),
                    group: entry.group,
                    edits: entry.edits,
                })
                .collect()
        };
        crate::host::HistoryView {
            undo: rows(&self.undo),
            redo: rows(&self.redo),
            activity: self
                .activity
                .iter()
                .map(|row| crate::host::ActivityView {
                    id: row.id,
                    at: row.at,
                    name: row.name.to_owned(),
                    subject: row.subject.clone(),
                    author: row.author.clone(),
                })
                .collect(),
        }
    }

    /// Adds a setting changed outside the undo history to the activity log, the oldest row
    /// dropped past [`MAX_ACTIVITY`].
    pub fn log(&mut self, name: &'static str, subject: Option<Subject>, author: &Author, at: u64) {
        self.next_entry += 1;
        if self.activity.len() >= MAX_ACTIVITY {
            self.activity.remove(0);
        }
        self.activity.push(Activity {
            id: self.next_entry,
            at,
            name,
            subject,
            author: author.clone(),
        });
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
    ///
    /// `subject` is what the command acted on ([`crate::describe`]): a new entry takes it; a
    /// command of the same name that joins the top entry merges into it; a command of another
    /// name counts in [`Entry::others`].
    #[allow(clippy::too_many_arguments)]
    pub fn record(
        &mut self,
        meta: Meta,
        undo: &Undo<crate::Command>,
        author: &Author,
        group: Option<GroupId>,
        at: u64,
        subject: Option<Subject>,
    ) -> bool {
        match (meta.kind, undo) {
            (Kind::Edit, Undo::Inverse(inverse)) => {
                self.redo.clear();
                match self.undo.last_mut() {
                    Some(top) if group.is_some() && top.group == group => {
                        top.commands.push((**inverse).clone());
                        top.edits = top.edits.saturating_add(1);
                        if top.name == meta.name {
                            match (&mut top.subject, subject) {
                                (Some(mine), Some(theirs)) => mine.merge(&theirs),
                                // One command of the name said nothing: the count is unknown.
                                (mine, _) => *mine = None,
                            }
                        } else {
                            top.others = top.others.saturating_add(1);
                        }
                    }
                    _ => {
                        self.next_entry += 1;
                        self.undo.push(Entry {
                            id: self.next_entry,
                            at,
                            name: meta.name,
                            subject,
                            others: 0,
                            author: author.clone(),
                            group,
                            edits: 1,
                            commands: vec![(**inverse).clone()],
                        });
                    }
                }
                true
            }
            (Kind::Edit, Undo::None) if meta.destructive => {
                let changed = self.can_undo() || self.can_redo();
                self.group_emptied |= self.open_group.is_some();
                self.undo.clear();
                self.redo.clear();
                changed
            }
            (Kind::Setting, _) => {
                self.log(meta.name, subject, author, at);
                true
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
    if matches!(command, crate::Command::History(_)) {
        return command.apply(state, ctx);
    }
    let meta = command.meta();
    // Read before the command applies: a delete takes the names away.
    let described = matches!(meta.kind, Kind::Edit | Kind::Setting)
        .then(|| crate::describe::before(command, &state.project))
        .flatten();
    let mut outcome = command.apply(state, ctx)?;
    let subject = described.or_else(|| match meta.kind {
        Kind::Edit | Kind::Setting => crate::describe::after(&outcome.output, &state.project),
        _ => None,
    });
    let group = ctx
        .group
        .or_else(|| {
            state
                .history
                .open_group
                .as_ref()
                .filter(|(_, opener)| *opener == ctx.author)
                .map(|(group, _)| *group)
        })
        .or_else(|| {
            // An agent's turn: only for what becomes an entry, so that nothing else
            // touches the turn (the module's rule).
            let entry = meta.kind == Kind::Edit && matches!(outcome.undo, Undo::Inverse(_));
            (entry && matches!(ctx.author, Author::Agent { .. }))
                .then(|| state.history.turn_group(&ctx.author, ctx.now_ms))
        });
    // A setting that changed nothing (no event) is not logged.
    if meta.kind == Kind::Setting && outcome.events.is_empty() {
        return Ok(outcome);
    }
    if state
        .history
        .record(meta, &outcome.undo, &ctx.author, group, ctx.now_ms, subject)
    {
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
            state.history.group_redo = state.history.redo.clone();
            state.history.group_emptied = false;
            Ok(live(Output::BeginGroup { group }))
        }
        Command::AbortGroup {} => abort_group(state, ctx),
        Command::EndGroup {} => match &state.history.open_group {
            Some((group, opener)) if *opener == ctx.author => {
                let group = *group;
                state.history.open_group = None;
                state.history.group_redo.clear();
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

/// `history.abort_group`: undoes the open group's entries and drops them, atomically.
fn abort_group(
    state: &mut State,
    ctx: &mut Context,
) -> Result<Outcome<Command, Output>, CommandError> {
    let invalid = |message: String| CommandError::Invalid {
        field: "group".into(),
        message,
    };
    let (group, opener) = match &state.history.open_group {
        Some(open) => open.clone(),
        None => return Err(invalid("no history group is open".into())),
    };
    if opener != ctx.author {
        return Err(invalid(format!(
            "group {} was opened by {}; only its author aborts it",
            group.0,
            String::from(opener)
        )));
    }
    // The group's entries must be the top of the undo stack: an entry of another author
    // under or between them would be undone out of order.
    let total = state
        .history
        .undo
        .iter()
        .filter(|entry| entry.group == Some(group))
        .count();
    let on_top = state
        .history
        .undo
        .iter()
        .rev()
        .take_while(|entry| entry.group == Some(group))
        .count();
    if on_top != total {
        return Err(invalid(format!(
            "another edit landed inside group {}; it cannot be taken back",
            group.0
        )));
    }
    if state.history.group_emptied {
        return Err(invalid(format!(
            "an irreversible edit emptied the history inside group {}; it cannot be taken back",
            group.0
        )));
    }
    let saved = (
        state.project.clone(),
        state.session.clone(),
        ctx.engine.clone(),
    );
    let mut taken = Vec::with_capacity(on_top);
    let mut events = Vec::new();
    let mut control = Vec::new();
    let mut undone = 0u32;
    for _ in 0..on_top {
        let entry = state.history.undo.pop().expect("counted above");
        match apply_entry(&entry, state, ctx) {
            Ok((_, e, c)) => {
                events.extend(e);
                control.extend(c);
                undone = undone.saturating_add(entry.edits);
                taken.push(entry);
            }
            Err(error) => {
                // As it was: the project, the session, the engine's slot table and the stack.
                (state.project, state.session, ctx.engine) = saved;
                state.history.undo.push(entry);
                while let Some(entry) = taken.pop() {
                    state.history.undo.push(entry);
                }
                return Err(error);
            }
        }
    }
    state.history.open_group = None;
    state.history.redo = std::mem::take(&mut state.history.group_redo);
    // An agent's turn never continues over entries that are gone.
    state.history.turn = None;
    if on_top > 0 {
        events.push(state.history.changed_event());
    }
    Ok(Outcome {
        output: Output::AbortGroup { group, undone },
        undo: Undo::None,
        events,
        control,
    })
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
    // An undo or a redo ends the agent's turn: its next edit never joins what was undone
    // or redone, even when the same entry is back on top.
    state.history.turn = None;
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
