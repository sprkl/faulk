//! `arrangement.key_change.*`: the key changes of the timeline (#953): set, move, remove and
//! list. Declared in the arrangement domain (`arrangement.rs`, the sixteen domains are
//! frozen); applied here. From the bar that starts at a change's tick the project is in its
//! key, until the next change; the starting key is `project.set_scale`'s.
//!
//! The contract, repeated in the descriptions for agents:
//!
//! - **Key changes are `edit`s**: undoable and saved with the project (`project.json`).
//! - A change stands at the start of a bar after the first (a positive multiple of the bar
//!   length of the project's time signature), at most 2^40 ticks; one change per tick;
//!   at most [`MAX_KEY_CHANGES`].
//! - A change is identified by its tick. `set` on an occupied tick replaces it; `move` onto
//!   an occupied tick is refused, so that every undo is a plain inverse command.
//! - Time edits (`arrangement.insert_time`, `delete_time`) do not move them.
//! - A command that changes nothing returns no inverse and no event.

use crate::arrangement::{Command, Output};
use crate::contract::{CommandError, Outcome, ProjectEvent, State, Undo};
use project::limits::{MAX_KEY_CHANGES, MAX_TICK};
use project::{KeyChange, Scale, ScaleKind, Tick};

/// A key change as `arrangement.key_change.list` answers it.
#[derive(
    Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct KeyChangeInfo {
    /// Where it stands, in ticks.
    pub tick: Tick,
    /// The bar it starts, 1 being the first.
    pub bar: u32,
    /// The tonic, 0 (C) to 11 (B); 0 for no scale.
    pub root: u8,
    /// The scale; `chromatic` for none.
    pub mode: ScaleKind,
    /// The pitch classes of the scale (0 = C to 11 = B), rising; all twelve for none.
    pub pitch_classes: Vec<u8>,
}

type Out = Outcome<Command, Output>;

fn invalid(field: &str, message: impl Into<String>) -> CommandError {
    CommandError::Invalid {
        field: field.into(),
        message: message.into(),
    }
}

/// The scale a command names: `chromatic` is none.
fn scale_of(root: u8, mode: ScaleKind) -> Result<Option<Scale>, CommandError> {
    if root > project::scale::MAX_ROOT {
        return Err(invalid(
            "root",
            format!("a root is 0 (C) to 11 (B), not {root}"),
        ));
    }
    Ok(mode.mode().map(|mode| Scale { root, mode }))
}

/// A change's scale as the commands name it.
fn kind_of(scale: Option<Scale>) -> (u8, ScaleKind) {
    crate::project::kind_of(scale)
}

/// The tick must be the start of a bar after the first.
fn check_tick(state: &State, field: &str, tick: Tick) -> Result<(), CommandError> {
    let bar = state.project.time_signature().ticks_per_bar().get();
    if tick.get() == 0 {
        return Err(invalid(
            field,
            "the starting key is not a key change: set it with project.set_scale",
        ));
    }
    if tick.get() < 0 || tick.get() > MAX_TICK || bar <= 0 || tick.get() % bar != 0 {
        return Err(invalid(
            field,
            format!(
                "a key change is at the start of a bar: a multiple of {bar} ticks, up to {MAX_TICK}, not {}",
                tick.get()
            ),
        ));
    }
    Ok(())
}

fn not_found(tick: Tick) -> CommandError {
    CommandError::NotFound {
        entity: "key change".into(),
        id: tick.get().to_string(),
    }
}

fn edited(output: Output, inverse: Command) -> Out {
    Outcome {
        output,
        undo: Undo::Inverse(Box::new(inverse)),
        events: vec![ProjectEvent::KeyChangesChanged {}],
        control: vec![],
    }
}

fn unchanged(output: Output) -> Out {
    Outcome {
        output,
        undo: Undo::None,
        events: vec![],
        control: vec![],
    }
}

/// Puts a change in its place in the sorted list, replacing the one at that tick; answers
/// the one replaced.
fn put(state: &mut State, change: KeyChange) -> Option<KeyChange> {
    let list = &mut state.project.key_changes;
    match list.binary_search_by_key(&change.tick, |c| c.tick) {
        Ok(i) => Some(std::mem::replace(&mut list[i], change)),
        Err(i) => {
            list.insert(i, change);
            None
        }
    }
}

fn take(state: &mut State, tick: Tick) -> Option<KeyChange> {
    let list = &mut state.project.key_changes;
    let i = list.binary_search_by_key(&tick, |c| c.tick).ok()?;
    Some(list.remove(i))
}

fn set_command(tick: Tick, scale: Option<Scale>) -> Command {
    let (root, mode) = kind_of(scale);
    Command::SetKeyChange { tick, root, mode }
}

/// Applies a key change command of the arrangement domain; `None` for any other command.
pub(crate) fn apply(command: &Command, state: &mut State) -> Option<Result<Out, CommandError>> {
    matches!(
        command,
        Command::SetKeyChange { .. }
            | Command::MoveKeyChange { .. }
            | Command::RemoveKeyChange { .. }
            | Command::ListKeyChanges {}
    )
    .then(|| run(command, state))
}

fn run(command: &Command, state: &mut State) -> Result<Out, CommandError> {
    match command {
        Command::SetKeyChange { tick, root, mode } => {
            check_tick(state, "tick", *tick)?;
            let scale = scale_of(*root, *mode)?;
            let existing = state
                .project
                .key_changes
                .iter()
                .find(|c| c.tick == *tick)
                .copied();
            if existing.is_none() && state.project.key_changes.len() >= MAX_KEY_CHANGES {
                return Err(CommandError::Limit {
                    entity: "key changes".into(),
                    max: MAX_KEY_CHANGES as u64,
                });
            }
            let (previous_root, previous_mode) = kind_of(existing.and_then(|c| c.scale));
            let output = Output::SetKeyChange {
                replaced: existing.is_some(),
                previous_root,
                previous_mode,
            };
            if existing.is_some_and(|c| c.scale == scale) {
                return Ok(unchanged(output));
            }
            put(state, KeyChange { tick: *tick, scale });
            let inverse = match existing {
                Some(old) => set_command(*tick, old.scale),
                None => Command::RemoveKeyChange { tick: *tick },
            };
            Ok(edited(output, inverse))
        }
        Command::MoveKeyChange { from_tick, to_tick } => {
            // The change is found by its tick whatever the bar length is now (the time signature may
            // have changed since it was put); only where it goes must be a bar start.
            check_tick(state, "to_tick", *to_tick)?;
            let list = &state.project.key_changes;
            let Some(change) = list.iter().find(|c| c.tick == *from_tick).copied() else {
                return Err(not_found(*from_tick));
            };
            if from_tick == to_tick {
                return Ok(unchanged(Output::MoveKeyChange {}));
            }
            if list.iter().any(|c| c.tick == *to_tick) {
                return Err(invalid(
                    "to_tick",
                    "a key change already stands there: remove it first",
                ));
            }
            take(state, *from_tick);
            put(
                state,
                KeyChange {
                    tick: *to_tick,
                    scale: change.scale,
                },
            );
            Ok(edited(
                Output::MoveKeyChange {},
                Command::MoveKeyChange {
                    from_tick: *to_tick,
                    to_tick: *from_tick,
                },
            ))
        }
        Command::RemoveKeyChange { tick } => {
            let change = take(state, *tick).ok_or_else(|| not_found(*tick))?;
            let (root, mode) = kind_of(change.scale);
            Ok(edited(
                Output::RemoveKeyChange { root, mode },
                set_command(*tick, change.scale),
            ))
        }
        Command::ListKeyChanges {} => {
            let bar = state.project.time_signature().ticks_per_bar().get().max(1);
            Ok(crate::query::answer(Output::ListKeyChanges {
                key_changes: state
                    .project
                    .key_changes
                    .iter()
                    .map(|c| {
                        let (root, mode) = kind_of(c.scale);
                        KeyChangeInfo {
                            tick: c.tick,
                            bar: u32::try_from(c.tick.get() / bar + 1).unwrap_or(u32::MAX),
                            root,
                            mode,
                            pitch_classes: c
                                .scale
                                .map_or_else(|| (0..12).collect(), project::Scale::pitch_classes),
                        }
                    })
                    .collect(),
            }))
        }
        _ => Err(CommandError::Unsupported {
            command: command.name().into(),
        }),
    }
}
