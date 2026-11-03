//! One example of every query command (PR-6) and of its branches. `tests/it/contract.rs` adds
//! [`examples`] to the catalog of examples (a command without one fails there; the harness
//! checks that a query changes nothing); `tests/it/query.rs` checks the behaviour in depth.

#![allow(dead_code)]

use crate::automation_examples;
use crate::clip_examples;
use crate::marker_examples;
use crate::transport_examples;
use commands::{
    Command, State, arrangement, automation, clip, device, project, scene, track, transport,
};

pub fn examples() -> Vec<(Command, State)> {
    let lanes = automation_examples::fx();
    let clips = clip_examples::fx();
    let on_lanes = |c: Command| (c, lanes.state.clone());
    let on_clips = |c: Command| (c, clips.state.clone());
    vec![
        on_lanes(Command::Project(project::Command::Get {})),
        on_clips(Command::Project(project::Command::Get {})),
        on_clips(Command::Track(track::Command::List {
            offset: None,
            limit: None,
        })),
        on_clips(Command::Track(track::Command::List {
            offset: Some(1),
            limit: Some(1),
        })),
        on_clips(Command::Track(track::Command::Get {
            track: clips.track.clone(),
            offset: None,
            limit: None,
        })),
        on_clips(Command::Track(track::Command::Get {
            track: clips.audio_track.clone(),
            offset: Some(0),
            limit: Some(1),
        })),
        on_clips(Command::Clip(clip::Command::Get {
            clip: clips.clip.clone(),
            offset: None,
            limit: None,
        })),
        on_clips(Command::Clip(clip::Command::Get {
            clip: clips.placed.clone(),
            offset: None,
            limit: None,
        })),
        on_clips(Command::Clip(clip::Command::Get {
            clip: clips.audio_clip.clone(),
            offset: None,
            limit: None,
        })),
        on_clips(Command::Clip(clip::Command::NotesList {
            clip: clips.clip.clone(),
            offset: None,
            limit: None,
        })),
        on_clips(Command::Clip(clip::Command::NotesList {
            clip: clips.clip.clone(),
            offset: Some(1),
            limit: Some(1),
        })),
        on_clips(Command::Scene(scene::Command::List {
            offset: None,
            limit: None,
        })),
        on_lanes(Command::Device(device::Command::Describe {
            device: Some(lanes.chain.synth.clone()),
            kind: None,
        })),
        on_lanes(Command::Device(device::Command::Describe {
            device: Some(lanes.chain.ma.clone()),
            kind: None,
        })),
        on_lanes(Command::Device(device::Command::Describe {
            device: Some(lanes.chain.player.clone()),
            kind: None,
        })),
        on_lanes(Command::Device(device::Command::Describe {
            device: None,
            kind: Some(device::SYNTH_KIND.into()),
        })),
        on_lanes(Command::Automation(automation::Command::LaneList {
            track: None,
            master: false,
            offset: None,
            limit: None,
        })),
        on_lanes(Command::Automation(automation::Command::LaneList {
            track: Some(lanes.track.clone()),
            master: false,
            offset: Some(1),
            limit: Some(1),
        })),
        on_lanes(Command::Automation(automation::Command::LaneList {
            track: None,
            master: true,
            offset: None,
            limit: None,
        })),
        on_lanes(Command::Automation(automation::Command::LanePoints {
            lane: lanes.gain_lane.clone(),
            offset: None,
            limit: None,
        })),
        on_lanes(Command::Automation(automation::Command::LanePoints {
            lane: lanes.gain_lane.clone(),
            offset: Some(1),
            limit: Some(1),
        })),
        (
            Command::Transport(transport::Command::Get {}),
            transport_examples::looping(),
        ),
        on_clips(Command::Transport(transport::Command::Get {})),
        (
            Command::Arrangement(arrangement::Command::ListMarkers {}),
            marker_examples::fx().state,
        ),
        on_clips(Command::Arrangement(arrangement::Command::ListMarkers {})),
    ]
}
