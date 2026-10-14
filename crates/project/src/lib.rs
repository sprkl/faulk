//! Faulk project document (project model, `docs/architecture/02-modele-projet.md`).
//!
//! The page owns the project in both modes (web and Boost); the helper relays and never
//! owns it. This crate is pure Rust with no I/O: it holds the document ([`Project`] and
//! what it contains), the musical time contract (re-exported from `engine::time`, CT-2),
//! the stable identifiers ([`ids`], CT-4), the fractional order keys ([`order`]), the
//! blob references ([`Hash`]), the safety limits ([`limits`]), the invariants
//! ([`Project::validate`]) and the session state ([`SessionState`]), which is kept apart
//! from the document and never serialized with it (founder's decision 10).
//!
//! The canonical folder format (one file per track, device and clip, TSV for notes,
//! points and placements) is SV-1's; the serde derives here are the in-memory JSON shape
//! the round-trip tests and the later tasks build on.

pub mod blob;
pub mod ids;
pub mod limits;
pub mod model;
pub mod order;
pub mod serde_util;
pub mod session;
pub mod validate;

/// Musical time: the contract shared by the project and the engine (`engine::time`).
pub use engine::time;
pub use engine::time::{Frame, LoopRegion, PPQ, Quantize, SampleRate, Tempo, Tick, TimeSignature};

pub use blob::Hash;
pub use ids::{
    ClipId, DeviceId, Id, IdGenerator, LaneId, PlacementId, ProjectId, SceneId, TrackId,
};
pub use model::*;
pub use order::OrderKey;
pub use serde_util::Rgb;
pub use session::SessionState;
pub use validate::{Violation, ViolationKind};
