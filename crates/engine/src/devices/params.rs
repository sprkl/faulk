//! Constructors of parameter descriptors shared by the MIDI effects (#788): a `const` table of
//! `ParamDescriptor`s is written with these instead of a struct literal per parameter.

use crate::device::{ParamDescriptor, Scale, Unit};
use crate::msg::ParamId;

/// A choice parameter: one of `values`, the index being the value (`default` is an index).
pub const fn choice(
    id: u32,
    key: &'static str,
    description: &'static str,
    default: f64,
    values: &'static [&'static str],
) -> ParamDescriptor {
    ParamDescriptor {
        id: ParamId(id),
        key,
        description,
        unit: Unit::None,
        min: 0.0,
        max: (values.len() - 1) as f64,
        default,
        scale: Scale::Linear,
        step: 1.0,
        smoothing_seconds: 0.0,
        values,
    }
}

/// A linear parameter moving by `step` from `min` to `max`, not smoothed.
#[allow(clippy::too_many_arguments)]
pub const fn stepped(
    id: u32,
    key: &'static str,
    description: &'static str,
    unit: Unit,
    min: f64,
    max: f64,
    default: f64,
    step: f64,
) -> ParamDescriptor {
    ParamDescriptor {
        id: ParamId(id),
        key,
        description,
        unit,
        min,
        max,
        default,
        scale: Scale::Linear,
        step,
        smoothing_seconds: 0.0,
        values: &[],
    }
}
