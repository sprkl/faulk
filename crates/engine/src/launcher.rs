//! The launch grid in the engine (GR-1, `docs/specs/02-decoupage-mvp.md` § 2.2 and the GR-1
//! entry): what a scene launch, `StopAll` and `BackToArrangement` do to the tracks, and what
//! the engine reports of the grid in the `Feedback`. The per-track launch itself (boundary,
//! replacement, release of the old clip's notes) is [`crate::clip_player`]; this module
//! turns grid-wide orders into per-track ones and reads the players' state back.
//!
//! # Orders
//!
//! Every function runs in `Engine::apply`, at the start of a block, and only sets the
//! per-track launch state that each track's player takes at its first read:
//!
//! - [`launch_scene`]: each track whose row holds a clip in the scene gets the launch of
//!   that clip, all at the same boundary (every player reads the same first segment of the
//!   block). A track whose cell is empty is **stopped** at that same boundary when it plays
//!   or waits for a launch (a scene is a song section: afterwards exactly its clips play, and
//!   the model has no per-slot "keep playing" marker); a track that does nothing is left
//!   alone.
//! - [`stop_all`]: every track that plays, waits for a launch or has an order not yet taken
//!   gets a stop, at the boundary: it replaces a pending launch. Tracks that do nothing are
//!   left alone (they are not reported as stopping).
//! - [`back_to_arrangement`]: every track that plays, waits or has an order gets released
//!   **at once**, not at a boundary: the notes of its clip end at the first sample of the
//!   block, its pending launch is dropped, and the track's own timeline (the arrangement,
//!   [`crate::arrangement`]) plays again from the playhead. A track the grid holds without a
//!   clip (after a stop) goes back too.
//!
//! # The report
//!
//! [`active_clips`] lists the cells that are not idle, at most two per track:
//!
//! - the clip the track plays: [`ClipState::Playing`], or [`ClipState::Stopping`] when the
//!   order waiting for the track will leave it silent (a stop, a launch of an empty cell,
//!   of an audio clip on a MIDI track, or of a clip another track plays);
//! - the clip a launch waits to start: [`ClipState::Queued`] while the transport runs,
//!   [`ClipState::Armed`] while it is stopped (the order waits for the transport itself).
//!
//! A track the grid holds without a clip playing (after a stop, or when the clip ended; the
//! arrangement is silent) is reported as [`ClipState::Held`], in no cell ([`HELD_SCENE`]): the
//! page lights "back to arrangement" from it. A track that plays
//! a clip is held too, and says so by that clip.
//!
//! The order waiting for a track is the one not yet taken by its player (`Track::launch`,
//! the newest), else the one its player queued. The clip a track plays is reported in the
//! cell it sits in now; a clip cleared from its cell while it plays is reported as held.
//! Nothing here allocates or fails.

use crate::engine::{ClipContent, LaunchRequest, State, Track};
use crate::msg::{ActiveClip, ClipState, HELD_SCENE, SceneSlot, TrackKind, TrackSlot};
use crate::time::Tick;

/// Whether a track takes part in a stop: it plays, waits for a launch or has one.
fn engaged(track: &Track) -> bool {
    track.player.engaged() || track.launch.is_some() || track.back
}

/// A scene launch (module doc): the launch of the scene's clip on every track that has one.
pub fn launch_scene(state: &mut State, scene: SceneSlot) {
    for track in state.tracks.as_mut_slice().iter_mut().flatten() {
        let has_clip = track
            .scenes
            .as_slice()
            .get(scene.0 as usize)
            .is_some_and(Option::is_some);
        if has_clip {
            track.launch = Some(LaunchRequest::Clip(scene));
        } else if engaged(track) {
            track.launch = Some(LaunchRequest::Stop);
        }
    }
}

/// `StopAll` (module doc).
pub fn stop_all(state: &mut State) {
    for track in state.tracks.as_mut_slice().iter_mut().flatten() {
        if engaged(track) {
            track.launch = Some(LaunchRequest::Stop);
        }
    }
}

/// `BackToArrangement` (module doc).
pub fn back_to_arrangement(state: &mut State) {
    for track in state.tracks.as_mut_slice().iter_mut().flatten() {
        back_track(track);
    }
}

/// `BackTrackToArrangement`: the same release for one track; a track that follows its
/// arrangement is left alone.
pub fn back_track(track: &mut Track) {
    // A track the grid holds without a clip (after a stop) goes back too.
    if engaged(track) || track.player.taken() {
        track.launch = None;
        track.back = true;
    }
}

/// Whether `order` starts a clip on track `slot` (what the player's `start` accepts): the
/// cell holds a clip the track can play ([`plays_on`]) that no other track plays.
fn starts_a_clip(state: &State, slot: TrackSlot, track: &Track, scene: SceneSlot) -> bool {
    let Some(clip) = track
        .scenes
        .as_slice()
        .get(scene.0 as usize)
        .copied()
        .flatten()
        .and_then(|c| state.clips.get(c.0))
    else {
        return false;
    };
    plays_on(track.kind, &clip.content) && clip.on_track.is_none_or(|t| t == slot)
}

/// Whether a track of `kind` plays a clip of `content` from the grid: a MIDI clip on any track
/// (an audio track's notes reach no instrument), an audio clip on an audio track only (its
/// player is the audio track's instrument stage, AU-4).
pub fn plays_on(kind: TrackKind, content: &ClipContent) -> bool {
    match content {
        ClipContent::Notes(_) => true,
        ClipContent::Audio(_) => kind == TrackKind::Audio,
    }
}

/// The cells that are not idle (module doc), in track order, playing clip first. `running`
/// tells whether the transport runs.
pub fn active_clips(state: &State, running: bool, mut push: impl FnMut(ActiveClip)) {
    for (index, entry) in state.tracks.as_slice().iter().enumerate() {
        let Some(track) = entry.as_deref() else {
            continue;
        };
        let slot = TrackSlot(index as u32);
        let order = track
            .launch
            .or_else(|| track.player.queued().map(|q| q.order));
        let next = match order {
            Some(LaunchRequest::Clip(scene)) if starts_a_clip(state, slot, track, scene) => {
                Some(scene)
            }
            _ => None,
        };
        let mut reported = false;
        if let Some(playing) = track.player.playing() {
            // The clip is reported where it sits; one cleared from its cell is not.
            let cell = track
                .scenes
                .as_slice()
                .get(playing.scene.0 as usize)
                .copied()
                .flatten();
            if cell == Some(playing.clip) {
                reported = true;
                let leaves = order.is_some() && next.is_none();
                push(ActiveClip {
                    track: slot,
                    scene: playing.scene,
                    state: if leaves {
                        ClipState::Stopping
                    } else {
                        ClipState::Playing
                    },
                    anchor: playing.anchor,
                });
            }
        }
        // A track the grid holds with no clip to report (none plays, or the one that plays was
        // cleared from its cell): its arrangement is silent.
        if !reported && track.player.taken() {
            push(ActiveClip {
                track: slot,
                scene: HELD_SCENE,
                state: ClipState::Held,
                anchor: Tick(0),
            });
        }
        if let Some(scene) = next {
            push(ActiveClip {
                track: slot,
                scene,
                state: if running {
                    ClipState::Queued
                } else {
                    ClipState::Armed
                },
                // The boundary is known only for the order the player has taken; a newer order that
                // still waits in `track.launch` has none yet.
                anchor: match track.launch {
                    Some(_) => Tick(0),
                    None => track.player.queued().and_then(|q| q.at).unwrap_or(Tick(0)),
                },
            });
        }
    }
}
