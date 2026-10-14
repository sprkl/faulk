//! Faulk project document.
//!
//! The page owns the project in both modes (web and Boost); the helper relays and never
//! owns it. This crate is pure Rust with no I/O.
//!
//! Until PR-1 (the project model) lands, [`Project`] and [`SessionState`] are the minimal
//! placeholders the command contract (CT-4, `crates/commands::v2`) applies to: the shapes
//! below are the contract, the fields are the ones PR-1 keeps.

pub mod ids;

use serde::{Deserialize, Serialize};

pub use ids::{ClipId, DeviceId, Id, IdGenerator, LaneId, PlacementId, SceneId, TrackId};

/// The project document: what makes the music, saved and versioned.
///
/// Placeholder for PR-1: only the fields the command contract needs today.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Project {
    /// Display name of the project.
    pub name: String,
}

/// The session state: what belongs to the screen or the machine, not to the music
/// (founder's decision 10 of 2026-10-14). Written to `.faulk/`, ignored by git, never
/// versioned with the project. Commands change it like the project, but `live` commands
/// change only it.
///
/// Placeholder for PR-1: only the fields the command contract needs today.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SessionState {
    /// The selected track: where live MIDI goes. `None` when no track is selected.
    pub selected_track: Option<TrackId>,
}
