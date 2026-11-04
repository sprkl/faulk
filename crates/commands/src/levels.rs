//! What `mixer.levels` and `project.analyze_render` answer: the level of every track and of
//! the master, so that an agent with no ears can tell whether its song sounds.
//!
//! Both are host queries, like `project.list_stored`: the page measures (the live meters of
//! the engine's report, or an offline render of the range), stages the readings in the
//! [`crate::Context`], and the command turns them into its typed output, naming the tracks.
//! A staged reading serves one answer.
//!
//! Levels are in dBFS (0 is full scale), `null` for silence. A strip's peak is the larger of
//! its two channels; its RMS is the power mean of the two.

use project::{Project, Tick, TrackId};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The longest range `project.analyze_render` measures, in seconds of content: the render
/// has to finish within the page's time cap, an order of magnitude faster than real time.
pub const MAX_ANALYZE_SECS: u64 = 600;

/// A level at or under this (linear) is silence: -120 dBFS.
const FLOOR: f64 = 1e-6;

/// One meter, linear amplitude, as the engine reports it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MeterReading {
    pub peak_l: f32,
    pub peak_r: f32,
    pub rms_l: f32,
    pub rms_r: f32,
}

/// The live meters a host staged for `mixer.levels`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LevelsStage {
    /// Whether the transport was playing or recording when the readings were taken.
    pub playing: bool,
    /// By engine track slot; a slot beyond the list is silent.
    pub tracks: Vec<MeterReading>,
    pub master: MeterReading,
}

/// The measures a host staged for `project.analyze_render`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AnalysisStage {
    /// The range the host rendered, in ticks: the command refuses the readings when the
    /// project's range is no longer this one.
    pub start_tick: Tick,
    pub end_tick: Tick,
    /// By engine slot of the render (the slots `engine_msgs::snapshot` assigns).
    pub tracks: Vec<MeterReading>,
    pub master: MeterReading,
    /// Frames the output protection silenced for a non-finite sample.
    pub silenced_frames: u64,
    /// The output protection's peak gain reduction, linear in `[0, 1]`.
    pub reduction: f64,
}

/// A level in dBFS; `null` is silence.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Level {
    /// The largest sample of either channel, in dBFS.
    #[schemars(required, schema_with = "nullable_db")]
    pub peak_db: Option<f64>,
    /// The root mean square of both channels, in dBFS.
    #[schemars(required, schema_with = "nullable_db")]
    pub rms_db: Option<f64>,
}

/// The level of a track.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TrackLevel {
    pub track: TrackId,
    pub name: String,
    #[schemars(required, schema_with = "nullable_db")]
    pub peak_db: Option<f64>,
    #[schemars(required, schema_with = "nullable_db")]
    pub rms_db: Option<f64>,
}

fn nullable_db(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
    schemars::json_schema!({
        "type": ["number", "null"],
        "format": "double",
        "description": "A level in dBFS (0 is full scale, above 0 is over); `null` is silence."
    })
}

fn db(linear: f64) -> Option<f64> {
    (linear.is_finite() && linear > FLOOR).then(|| 20.0 * linear.log10())
}

/// The level of a meter reading: the larger peak, the power mean of the two RMS.
pub fn level_of(m: &MeterReading) -> Level {
    let peak = f64::from(m.peak_l).max(f64::from(m.peak_r));
    let rms = ((f64::from(m.rms_l).powi(2) + f64::from(m.rms_r).powi(2)) / 2.0).sqrt();
    Level {
        peak_db: db(peak),
        rms_db: db(rms),
    }
}

/// The levels of the project's tracks, in display order, from readings `tracks` by engine
/// slot (`slot_of` gives a track's slot); a track with no reading is silent.
pub fn track_levels(
    project: &Project,
    slot_of: impl Fn(&TrackId) -> Option<u32>,
    tracks: &[MeterReading],
) -> Vec<TrackLevel> {
    project
        .tracks_in_order()
        .into_iter()
        .map(|(id, track)| {
            let reading = slot_of(id).and_then(|slot| tracks.get(slot as usize));
            let level = reading.map(level_of).unwrap_or(Level {
                peak_db: None,
                rms_db: None,
            });
            TrackLevel {
                track: id.clone(),
                name: track.name.clone(),
                peak_db: level.peak_db,
                rms_db: level.rms_db,
            }
        })
        .collect()
}

/// The gain reduction of the output protection in dB (0 when it never engaged).
pub fn reduction_db(linear: f64) -> f64 {
    if linear.is_finite() && linear > 0.0 {
        -20.0 * (1.0 - linear.min(0.999_999)).log10()
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(p: f32, r: f32) -> MeterReading {
        MeterReading {
            peak_l: p,
            peak_r: p / 2.0,
            rms_l: r,
            rms_r: r,
        }
    }

    #[test]
    fn full_scale_is_zero_db_and_silence_is_null() {
        let l = level_of(&m(1.0, 0.5));
        assert_eq!(l.peak_db, Some(0.0));
        assert!((l.rms_db.unwrap() - 20.0 * 0.5f64.log10()).abs() < 1e-9);
        assert_eq!(level_of(&m(0.0, 0.0)).peak_db, None);
        assert_eq!(level_of(&m(f32::NAN, f32::INFINITY)).rms_db, None);
    }

    #[test]
    fn the_reduction_is_in_db() {
        assert_eq!(reduction_db(0.0), 0.0);
        assert!((reduction_db(0.5) - 6.0206).abs() < 1e-3);
        assert!(reduction_db(1.0).is_finite());
    }
}
