//! One example of every branch of each track command, with the state it runs from.
//! `tests/contract.rs` adds [`examples`] to the catalog of examples (a command without
//! one fails there); `tests/track.rs` checks the behaviour in depth.

#![allow(dead_code)]

use crate::clip_examples::{after, fx};
use ::project::{Rgb, TrackKind};
use commands::track;
use commands::{Command, State};

pub fn t(command: track::Command) -> Command {
    Command::Track(command)
}

/// The state with the first track coloured, so that clearing a colour changes something.
fn coloured(state: &State, track: &::project::TrackId) -> State {
    let mut s = state.clone();
    s.project.tracks.get_mut(track).unwrap().color = Some(Rgb::new(255, 0, 0));
    s
}

/// The state with the first track given a height, so that clearing it changes something.
fn tall(state: &State, track: &::project::TrackId) -> State {
    let mut s = state.clone();
    s.project.tracks.get_mut(track).unwrap().height = Some(96);
    s
}

pub fn examples() -> Vec<(Command, State)> {
    let f = fx();
    let on = |command: track::Command| (t(command), f.state.clone());
    let deleted = after(
        f.state.clone(),
        &t(track::Command::Delete {
            track: f.track.clone(),
        }),
    );
    let moved = after(
        f.state.clone(),
        &t(track::Command::Move {
            track: f.track.clone(),
            index: 1,
        }),
    );
    vec![
        on(track::Command::Create {
            kind: TrackKind::Midi,
            instrument: None,
            name: None,
            color: None,
        }),
        on(track::Command::Create {
            kind: TrackKind::Midi,
            instrument: Some("faulk.osc_synth".into()),
            name: Some("  Lead  ".into()),
            color: Some(Rgb::new(0, 128, 255)),
        }),
        on(track::Command::Create {
            kind: TrackKind::Audio,
            instrument: None,
            name: Some("Vox".into()),
            color: None,
        }),
        // A track with devices, clips and placements, then one with an audio clip.
        on(track::Command::Delete {
            track: f.track.clone(),
        }),
        on(track::Command::Delete {
            track: f.audio_track.clone(),
        }),
        // The inverse of a delete, from the state after it.
        (deleted.1, deleted.0),
        on(track::Command::Rename {
            track: f.track.clone(),
            name: "  Bass  ".into(),
        }),
        on(track::Command::Rename {
            track: f.track.clone(),
            name: String::new(),
        }),
        on(track::Command::SetColor {
            track: f.track.clone(),
            color: Some(Rgb::new(10, 20, 30)),
        }),
        (
            t(track::Command::SetColor {
                track: f.track.clone(),
                color: None,
            }),
            coloured(&f.state, &f.track),
        ),
        on(track::Command::SetHeight {
            track: f.track.clone(),
            height: Some(96),
        }),
        (
            t(track::Command::SetHeight {
                track: f.track.clone(),
                height: None,
            }),
            tall(&f.state, &f.track),
        ),
        on(track::Command::Move {
            track: f.track.clone(),
            index: 1,
        }),
        on(track::Command::Move {
            track: f.audio_track.clone(),
            index: 0,
        }),
        // The inverse of a move: the exact previous key.
        (moved.1, moved.0),
        on(track::Command::Duplicate {
            track: f.track.clone(),
            name: None,
        }),
        on(track::Command::Duplicate {
            track: f.audio_track.clone(),
            name: Some("Drums 2".into()),
        }),
        on(track::Command::Arm {
            track: f.track.clone(),
            armed: true,
        }),
        on(track::Command::Arm {
            track: f.audio_track.clone(),
            armed: false,
        }),
        on(track::Command::SetMonitor {
            track: f.track.clone(),
            monitor: true,
        }),
    ]
}
