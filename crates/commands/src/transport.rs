//! `transport.*`: playback and the saved transport settings: play, stop, position, tempo,
//! time signature, loop region and metronome (epic 3, TR).
//!
//! Contract of the values (TR-3):
//!
//! - **`play`, `stop` and `seek` are `live`**: they change neither the project nor the
//!   session and never enter the history. `apply` only checks `seek`'s position and
//!   returns no engine message: until PR-3 gives the control messages (as for `live.*`),
//!   nothing plays, and each host must translate them to the engine itself.
//! - **Tempo, time signature, loop and metronome are `edit`s** in the project: undoable
//!   and saved. A tempo outside [`Tempo::MIN`], [`Tempo::MAX`] BPM is refused, never
//!   clamped. One tempo and one time signature per project in the MVP: the commands set
//!   the first point of the maps.
//! - The loop region needs `0 <= start < end <= MAX_TICK`. The loop cannot be enabled
//!   without a region, and the region cannot be cleared while the loop is enabled
//!   (the project would be invalid): turn the loop off first.
//! - A command that changes nothing returns no inverse and no event.

use crate::contract::{CommandError, Context, Outcome, ProjectEvent, State, Undo};
use crate::macros::domain;
use project::limits::{MAX_COUNT_IN_BARS, MAX_TICK};
use project::{LoopRegion, Tempo, Tick, TickLoop, TimeSignature};

/// The lowest finite metronome level a command accepts, in dB.
pub const MIN_METRONOME_DB: f64 = -96.0;
/// The highest metronome level a command accepts, in dB.
pub const MAX_METRONOME_DB: f64 = 6.0;

domain! {
    domain = "transport", schema = ("TransportCommand", "TransportOutput");

    /// Start playback from the current position. Played live: not undoable, not saved.
    "transport.play" (kind = "live", undo = "none", destructive = false)
    Play {} => {};

    /// Stop playback. Played live: not undoable, not saved.
    "transport.stop" (kind = "live", undo = "none", destructive = false)
    Stop {} => {};

    /// Move the playhead to `tick` (960 ticks per quarter note), from 0 to 2^40. Played
    /// live: not undoable, not saved.
    "transport.seek" (kind = "live", undo = "none", destructive = false)
    Seek {
        tick: Tick,
    } => {};

    /// Set the project's tempo in BPM, from 20 to 999. A value outside the range or not
    /// a number is refused. Undo: set the previous tempo back.
    "transport.set_tempo" (kind = "edit", undo = "inverse", destructive = false)
    SetTempo {
        #[schemars(range(min = 20.0, max = 999.0))]
        bpm: f64,
    } => {
        /// The tempo before the command, in BPM.
        previous: f64,
    };

    /// Set the project's time signature: numerator 1 to 32, denominator a power of two
    /// from 1 to 32 (4 and 4 is common time). Anything else is refused. Undo: set the
    /// previous signature back.
    "transport.set_time_signature" (kind = "edit", undo = "inverse", destructive = false)
    SetTimeSignature {
        #[schemars(range(min = 1, max = 32))]
        numerator: u8,
        #[schemars(range(min = 1, max = 32))]
        denominator: u8,
    } => {
        /// The numerator before the command.
        previous_numerator: u8,
        /// The denominator before the command.
        previous_denominator: u8,
    };

    /// Set the loop region `{"start": <tick>, "end": <tick>}` with `0 <= start < end <=
    /// 2^40`, or `null` for none; a region cannot be cleared while the loop is on. This
    /// does not turn the loop on: see `transport.toggle_loop`. Undo: set the previous
    /// region back.
    "transport.set_loop" (kind = "edit", undo = "inverse", destructive = false)
    SetLoop {
        loop_region: Option<TickLoop>,
    } => {
        /// The region before the command.
        previous: Option<TickLoop>,
    };

    /// Turn the loop over the loop region on or off. Turning it on without a region is
    /// refused. Undo: toggle again.
    "transport.toggle_loop" (kind = "edit", undo = "inverse", destructive = false)
    ToggleLoop {} => {
        /// Whether the loop is on after the command.
        enabled: bool,
    };

    /// Change the metronome: `enabled` (click on or off), `gain_db` (level, -96 to +6,
    /// `null` for silence) and `count_in_bars` (bars counted in before recording, 0 to 16). Every field is
    /// optional and a missing one is left as it is; at least one is required. A value
    /// outside its range is refused. Undo: set the previous values of the changed fields
    /// back.
    "transport.set_metronome" (kind = "edit", undo = "inverse", destructive = false)
    SetMetronome {
        #[serde(default)]
        enabled: Option<bool>,
        #[serde(
            default,
            deserialize_with = "present",
            skip_serializing_if = "Option::is_none"
        )]
        #[schemars(schema_with = "optional_gain")]
        gain_db: Option<Option<f64>>,
        #[serde(default)]
        #[schemars(range(min = 0, max = 16))]
        count_in_bars: Option<u8>,
    } => {
        /// Whether the click was on before the command.
        previous_enabled: bool,
        /// The level before the command, in dB; `null` was silence.
        #[schemars(required, schema_with = "nullable_number")]
        previous_gain_db: Option<f64>,
        /// The count-in before the command, in bars.
        previous_count_in_bars: u8,
    };
}

type Out = Outcome<Command, Output>;

fn invalid(field: &str, message: String) -> CommandError {
    CommandError::Invalid {
        field: field.into(),
        message,
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

fn edited(output: Output, inverse: Command, event: ProjectEvent) -> Out {
    Outcome {
        output,
        undo: Undo::Inverse(Box::new(inverse)),
        events: vec![event],
        control: vec![],
    }
}

/// Applies a command of this domain.
pub fn apply(
    command: &Command,
    state: &mut State,
    _ctx: &mut Context,
) -> Result<Out, CommandError> {
    match command {
        Command::Play {} => Ok(unchanged(Output::Play {})),
        Command::Stop {} => Ok(unchanged(Output::Stop {})),
        Command::Seek { tick } => {
            if !(0..=MAX_TICK).contains(&tick.get()) {
                return Err(invalid(
                    "tick",
                    format!("a position is 0 to {MAX_TICK} ticks, not {}", tick.get()),
                ));
            }
            Ok(unchanged(Output::Seek {}))
        }
        Command::SetTempo { bpm } => set_tempo(state, *bpm),
        Command::SetTimeSignature {
            numerator,
            denominator,
        } => set_time_signature(state, *numerator, *denominator),
        Command::SetLoop { loop_region } => set_loop(state, *loop_region),
        Command::ToggleLoop {} => toggle_loop(state),
        Command::SetMetronome {
            enabled,
            gain_db,
            count_in_bars,
        } => set_metronome(state, *enabled, *gain_db, *count_in_bars),
    }
}

fn set_tempo(state: &mut State, bpm: f64) -> Result<Out, CommandError> {
    let new = Tempo::new(bpm).ok_or_else(|| {
        invalid(
            "bpm",
            format!("a tempo is {} to {} BPM, not {bpm}", Tempo::MIN, Tempo::MAX),
        )
    })?;
    let point = state
        .project
        .tempo
        .first_mut()
        .ok_or_else(|| invalid("bpm", "the project has no tempo point".into()))?;
    let previous = point.bpm;
    let output = Output::SetTempo {
        previous: previous.bpm(),
    };
    if previous == new {
        return Ok(unchanged(output));
    }
    point.bpm = new;
    Ok(edited(
        output,
        Command::SetTempo {
            bpm: previous.bpm(),
        },
        ProjectEvent::TempoChanged { bpm: new.bpm() },
    ))
}

fn set_time_signature(
    state: &mut State,
    numerator: u8,
    denominator: u8,
) -> Result<Out, CommandError> {
    let new = TimeSignature::new(numerator, denominator).ok_or_else(|| {
        invalid(
            "time_signature",
            format!(
                "a numerator is 1 to 32 and a denominator a power of two from 1 to 32, not {numerator}/{denominator}"
            ),
        )
    })?;
    let point = state
        .project
        .meter
        .first_mut()
        .ok_or_else(|| invalid("time_signature", "the project has no meter point".into()))?;
    let previous = point.signature;
    let output = Output::SetTimeSignature {
        previous_numerator: previous.numerator(),
        previous_denominator: previous.denominator(),
    };
    if previous == new {
        return Ok(unchanged(output));
    }
    point.signature = new;
    Ok(edited(
        output,
        Command::SetTimeSignature {
            numerator: previous.numerator(),
            denominator: previous.denominator(),
        },
        ProjectEvent::TimeSignatureChanged {
            numerator,
            denominator,
        },
    ))
}

fn as_tick_loop(region: Option<LoopRegion>) -> Option<TickLoop> {
    region.map(|r| TickLoop {
        start: r.start(),
        end: r.end(),
    })
}

fn loop_changed(state: &State) -> ProjectEvent {
    let t = &state.project.transport;
    ProjectEvent::LoopChanged {
        region: as_tick_loop(t.loop_region),
        enabled: t.loop_enabled,
    }
}

fn set_loop(state: &mut State, new: Option<TickLoop>) -> Result<Out, CommandError> {
    let region = match new {
        None => None,
        Some(l) => {
            if l.start.get() < 0 || l.end.get() > MAX_TICK {
                return Err(invalid(
                    "loop_region",
                    format!(
                        "a loop is within 0 to {MAX_TICK} ticks, not {} to {}",
                        l.start.get(),
                        l.end.get()
                    ),
                ));
            }
            Some(LoopRegion::new(l.start, l.end).ok_or_else(|| {
                invalid(
                    "loop_region",
                    format!(
                        "a loop needs start < end, not {} to {}",
                        l.start.get(),
                        l.end.get()
                    ),
                )
            })?)
        }
    };
    let t = &mut state.project.transport;
    let previous = as_tick_loop(t.loop_region);
    let output = Output::SetLoop { previous };
    if t.loop_region == region {
        return Ok(unchanged(output));
    }
    if region.is_none() && t.loop_enabled {
        return Err(invalid(
            "loop_region",
            "the loop is on: turn it off (transport.toggle_loop) before clearing its region".into(),
        ));
    }
    t.loop_region = region;
    Ok(edited(
        output,
        Command::SetLoop {
            loop_region: previous,
        },
        loop_changed(state),
    ))
}

fn toggle_loop(state: &mut State) -> Result<Out, CommandError> {
    let t = &mut state.project.transport;
    if !t.loop_enabled && t.loop_region.is_none() {
        return Err(invalid(
            "loop_region",
            "no loop region is set: set one (transport.set_loop) before turning the loop on".into(),
        ));
    }
    t.loop_enabled = !t.loop_enabled;
    let enabled = t.loop_enabled;
    Ok(edited(
        Output::ToggleLoop { enabled },
        Command::ToggleLoop {},
        loop_changed(state),
    ))
}

fn db_in_range(db: f64) -> bool {
    db.is_finite() && (MIN_METRONOME_DB..=MAX_METRONOME_DB).contains(&db)
}

fn from_db(db: f64) -> Option<f64> {
    (db != f64::NEG_INFINITY).then_some(db)
}

fn set_metronome(
    state: &mut State,
    enabled: Option<bool>,
    gain_db: Option<Option<f64>>,
    count_in_bars: Option<u8>,
) -> Result<Out, CommandError> {
    if enabled.is_none() && gain_db.is_none() && count_in_bars.is_none() {
        return Err(invalid(
            "enabled",
            "give at least one of enabled, gain_db or count_in_bars".into(),
        ));
    }
    // The document's level: `null` is silence (-inf).
    let gain_db = match gain_db {
        None => None,
        Some(None) => Some(f64::NEG_INFINITY),
        Some(Some(g)) if db_in_range(g) => Some(g),
        Some(Some(g)) => {
            return Err(invalid(
                "gain_db",
                format!(
                    "a metronome level is {MIN_METRONOME_DB} to {MAX_METRONOME_DB} dB, or null for silence, not {g}"
                ),
            ));
        }
    };
    if let Some(n) = count_in_bars
        && n > MAX_COUNT_IN_BARS
    {
        return Err(invalid(
            "count_in_bars",
            format!("a count-in is 0 to {MAX_COUNT_IN_BARS} bars, not {n}"),
        ));
    }
    let m = &mut state.project.transport.metronome;
    // An inverse must restore the level exactly, and a command writes only a level in
    // range or silence: a finite level outside it (a document edited by hand) is not
    // changed by command.
    let restorable = m.gain_db == f64::NEG_INFINITY || db_in_range(m.gain_db);
    if gain_db.is_some_and(|g| g != m.gain_db) && !restorable {
        return Err(invalid(
            "gain_db",
            format!(
                "the current level, {} dB, is outside {MIN_METRONOME_DB} to {MAX_METRONOME_DB} dB: a command could not undo the edit exactly",
                m.gain_db
            ),
        ));
    }
    let output = Output::SetMetronome {
        previous_enabled: m.enabled,
        previous_gain_db: from_db(m.gain_db),
        previous_count_in_bars: m.count_in_bars,
    };
    // Only what changes goes into the inverse.
    let (mut inv_enabled, mut inv_gain, mut inv_count) = (None, None, None);
    if let Some(e) = enabled
        && e != m.enabled
    {
        inv_enabled = Some(m.enabled);
        m.enabled = e;
    }
    if let Some(g) = gain_db
        && g != m.gain_db
    {
        inv_gain = Some(from_db(m.gain_db));
        m.gain_db = g;
    }
    if let Some(n) = count_in_bars
        && n != m.count_in_bars
    {
        inv_count = Some(m.count_in_bars);
        m.count_in_bars = n;
    }
    if inv_enabled.is_none() && inv_gain.is_none() && inv_count.is_none() {
        return Ok(unchanged(output));
    }
    let event = ProjectEvent::MetronomeChanged {
        enabled: m.enabled,
        gain_db: from_db(m.gain_db),
        count_in_bars: m.count_in_bars,
    };
    Ok(edited(
        output,
        Command::SetMetronome {
            enabled: inv_enabled,
            gain_db: inv_gain,
            count_in_bars: inv_count,
        },
        event,
    ))
}

/// Distinguishes a missing field (`None`: leave as it is) from an explicit `null`
/// (`Some(None)`: silence).
fn present<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Option<f64>>, D::Error> {
    <Option<f64> as serde::Deserialize>::deserialize(d).map(Some)
}

/// The schema of the level to set: a number from [`MIN_METRONOME_DB`] to
/// [`MAX_METRONOME_DB`], or `null` for silence; the field itself is optional.
fn optional_gain(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
    schemars::json_schema!({
        "type": ["number", "null"],
        "format": "double",
        "minimum": MIN_METRONOME_DB,
        "maximum": MAX_METRONOME_DB,
        "description": "The level in dB; `null` is silence; missing leaves it as it is."
    })
}

/// The schema of a previous level: a number, or `null` when it was silence.
fn nullable_number(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
    schemars::json_schema!({
        "type": ["number", "null"],
        "format": "double",
        "description": "The level before the command, in dB; `null` was silence."
    })
}
