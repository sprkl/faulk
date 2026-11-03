//! `arrangement.marker.*`: the named section markers of the arrangement (#473): add,
//! rename, move, delete and the restore that undoes a delete. Declared in the arrangement
//! domain (`arrangement.rs`, the sixteen domains are frozen); applied here. A marker is a named point of the timeline
//! ("Verse", "Chorus"): the arrangement draws it on its ruler and the keyboard jumps from
//! one to the next.
//!
//! The contract of the marker commands, repeated in their descriptions for agents:
//!
//! - **Markers are `edit`s**: undoable and saved with the project (`project.json`).
//! - A tick is in `[0, 2^40]`; a name is trimmed, not empty, at most 200 characters. Two
//!   markers may stand on the same tick.
//! - A section marker is not the transport's cue (the point `transport.seek` sets and
//!   `transport.stop` goes back to): it is a named point the user places; jumping to one is a
//!   `transport.seek` to its tick.
//! - Inserting or deleting time (`arrangement.insert_time`, `arrangement.delete_time`) moves
//!   the markers after the point like the placements; a marker inside a deleted range goes
//!   to its start.
//! - A command that changes nothing returns no inverse and no event.

use crate::contract::{CommandError, Context, Outcome, ProjectEvent, State, Undo};
use project::limits::{MAX_MARKERS, MAX_NAME_CHARS, MAX_TICK};
use project::{Marker, MarkerId, Tick};

/// A section marker as `arrangement.marker.list` answers it.
#[derive(
    Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct MarkerInfo {
    pub marker: MarkerId,
    pub tick: Tick,
    pub name: String,
}

/// The prefix of a marker's default name ("Section 3").
pub const DEFAULT_PREFIX: &str = "Section";

use crate::arrangement::{Command, Output};

type Out = Outcome<Command, Output>;

fn invalid(field: &str, message: impl Into<String>) -> CommandError {
    CommandError::Invalid {
        field: field.into(),
        message: message.into(),
    }
}

fn not_found(id: &MarkerId) -> CommandError {
    CommandError::NotFound {
        entity: "marker".into(),
        id: id.to_string(),
    }
}

fn check_tick(tick: Tick) -> Result<(), CommandError> {
    if (0..=MAX_TICK).contains(&tick.get()) {
        Ok(())
    } else {
        Err(invalid(
            "tick",
            format!("a marker is at 0 to {MAX_TICK} ticks, not {}", tick.get()),
        ))
    }
}

fn clean_name(name: &str) -> Result<&str, CommandError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(invalid("name", "a marker's name is not empty"));
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(invalid(
            "name",
            format!("a marker's name has at most {MAX_NAME_CHARS} characters"),
        ));
    }
    Ok(name)
}

/// "Section n", n the first number no marker of the project has in that exact spelling.
fn default_name(state: &State) -> String {
    let used: std::collections::BTreeSet<u32> = state
        .project
        .markers
        .values()
        .filter_map(|m| {
            m.name
                .strip_prefix(DEFAULT_PREFIX)?
                .strip_prefix(' ')?
                .parse()
                .ok()
        })
        .collect();
    let n = (1u32..).find(|n| !used.contains(n)).unwrap_or(u32::MAX);
    format!("{DEFAULT_PREFIX} {n}")
}

fn edited(output: Output, inverse: Command) -> Out {
    Outcome {
        output,
        undo: Undo::Inverse(Box::new(inverse)),
        events: vec![ProjectEvent::MarkersChanged {}],
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

/// Puts a marker in, after the checks every way in shares.
fn insert(state: &mut State, id: MarkerId, tick: Tick, name: &str) -> Result<(), CommandError> {
    check_tick(tick)?;
    let name = clean_name(name)?;
    if state.project.markers.len() >= MAX_MARKERS {
        return Err(CommandError::Limit {
            entity: "markers".into(),
            max: MAX_MARKERS as u64,
        });
    }
    state.project.markers.insert(
        id,
        Marker {
            tick,
            name: name.to_owned(),
        },
    );
    Ok(())
}

/// Applies a marker command of the arrangement domain; `None` for any other command.
pub(crate) fn apply(
    command: &Command,
    state: &mut State,
    ctx: &mut Context,
) -> Option<Result<Out, CommandError>> {
    matches!(
        command,
        Command::AddMarker { .. }
            | Command::RenameMarker { .. }
            | Command::MoveMarker { .. }
            | Command::DeleteMarker { .. }
            | Command::RestoreMarker { .. }
            | Command::ListMarkers {}
    )
    .then(|| run_marker(command, state, ctx))
}

fn run_marker(
    command: &Command,
    state: &mut State,
    ctx: &mut Context,
) -> Result<Out, CommandError> {
    match command {
        Command::AddMarker { tick, name } => {
            let name = match name.as_deref().map(str::trim) {
                Some(n) if !n.is_empty() => n.to_owned(),
                _ => default_name(state),
            };
            let id = MarkerId::generate(&mut ctx.ids);
            insert(state, id.clone(), *tick, &name)?;
            Ok(edited(
                Output::AddMarker { marker: id.clone() },
                Command::DeleteMarker { marker: id },
            ))
        }
        Command::RenameMarker { marker, name } => {
            let name = clean_name(name)?.to_owned();
            let m = state
                .project
                .markers
                .get_mut(marker)
                .ok_or_else(|| not_found(marker))?;
            let previous = m.name.clone();
            let output = Output::RenameMarker {
                previous: previous.clone(),
            };
            if previous == name {
                return Ok(unchanged(output));
            }
            m.name = name;
            Ok(edited(
                output,
                Command::RenameMarker {
                    marker: marker.clone(),
                    name: previous,
                },
            ))
        }
        Command::MoveMarker { marker, tick } => {
            check_tick(*tick)?;
            let m = state
                .project
                .markers
                .get_mut(marker)
                .ok_or_else(|| not_found(marker))?;
            let previous = m.tick;
            let output = Output::MoveMarker { previous };
            if previous == *tick {
                return Ok(unchanged(output));
            }
            m.tick = *tick;
            Ok(edited(
                output,
                Command::MoveMarker {
                    marker: marker.clone(),
                    tick: previous,
                },
            ))
        }
        Command::DeleteMarker { marker } => {
            let m = state
                .project
                .markers
                .remove(marker)
                .ok_or_else(|| not_found(marker))?;
            Ok(edited(
                Output::DeleteMarker {
                    tick: m.tick,
                    name: m.name.clone(),
                },
                Command::RestoreMarker {
                    marker: marker.clone(),
                    tick: m.tick,
                    name: m.name,
                },
            ))
        }
        Command::RestoreMarker { marker, tick, name } => {
            if state.project.contains_id(&marker.0) {
                return Err(invalid(
                    "marker",
                    format!("the identifier {marker} is already in use"),
                ));
            }
            insert(state, marker.clone(), *tick, name)?;
            Ok(edited(
                Output::RestoreMarker {},
                Command::DeleteMarker {
                    marker: marker.clone(),
                },
            ))
        }
        Command::ListMarkers {} => Ok(crate::query::answer(Output::ListMarkers {
            markers: state
                .project
                .markers_in_order()
                .into_iter()
                .map(|(id, m)| MarkerInfo {
                    marker: id.clone(),
                    tick: m.tick,
                    name: m.name.clone(),
                })
                .collect(),
        })),
        _ => Err(CommandError::Unsupported {
            command: command.name().into(),
        }),
    }
}
