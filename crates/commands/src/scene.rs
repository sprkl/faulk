//! `scene.*`: the scenes of the launch grid: create, delete, restore, rename, reorder
//! (epic 6, GR-2). A scene is a row of the grid; the clips of its slots (one per track)
//! are launched together by `launch.scene` (`launch.rs`).
//!
//! The contract of the scene commands, repeated in their descriptions for agents:
//!
//! - **Scenes are `edit`s**: undoable and saved with the project. `scene.delete` of a scene
//!   that holds clips is undoable and **not** destructive (the contract refuses an edit
//!   that is both, as for `track.delete`): it takes the clips of the scene's slots with it
//!   (and their placements in the arrangement, as `clip.delete` does), and its inverse
//!   `scene.restore` brings the scene and every clip back with their identifiers. There is
//!   no confirmation to ask: one `history.undo` puts everything back.
//! - A command that changes nothing returns no inverse and no event.
//! - Everything is checked before anything changes: a refusal leaves the project as it was.

use crate::contract::{CommandError, Context, Outcome, ProjectEvent, ScenePart, State, Undo};
use crate::macros::domain;
use project::limits::{MAX_NAME_CHARS, MAX_SCENES};
use project::{Clip, ClipId, OrderKey, Placement, PlacementId, Project, Scene, SceneId, TrackId};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A scene as `scene.restore` brings it back (everything but its identifier).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SceneSnapshot {
    pub name: String,
    pub order: OrderKey,
}

/// A clip of a deleted scene, with its identifier, its track and its placements in the
/// arrangement, as `scene.restore` brings it back. MIDI or audio: it is the clip as the
/// project holds it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SceneClip {
    pub track: TrackId,
    pub id: ClipId,
    /// Its `slot` is the restored scene.
    pub clip: Clip,
    /// Its placements, by identifier.
    pub placements: Vec<ScenePlacement>,
}

/// A placement of a clip of a deleted scene, with its identifier.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScenePlacement {
    pub id: PlacementId,
    pub placement: Placement,
}

domain! {
    domain = "scene", schema = ("SceneCommand", "SceneOutput");

    /// Create a scene at the end of the launch grid. `name` is optional (trimmed, at most
    /// 200 characters; empty by default). The new scene's slots are empty. Refused with
    /// `limit` only at the project's safety limit of scenes. Undo: `scene.delete`.
    "scene.create" (kind = "edit", undo = "inverse", destructive = false)
    Create { name: Option<String> } => {
        /// The scene created.
        scene: SceneId,
    };

    /// Delete a scene with the clips of its launch-grid slots, and their placements in the
    /// arrangement. Not destructive: undo brings everything back. A clip that is playing
    /// stops sounding when its notes are released. Undo: `scene.restore` brings the scene
    /// and its clips back with their identifiers and position.
    "scene.delete" (kind = "edit", undo = "inverse", destructive = false)
    Delete { scene: SceneId } => {
        /// How many clips were deleted with the scene.
        clips: u32,
    };

    /// Bring a deleted scene back with its identifier, its name, its position and the
    /// clips of its slots, as `scene.delete` removed it: the inverse of a delete. Refused
    /// when the scene's identifier or any identifier inside it is already in use, at the
    /// safety limit of scenes, when a clip's slot is not this scene or two clips share a
    /// track, or when the snapshot is not a valid project. Undo: `scene.delete`.
    "scene.restore" (kind = "edit", undo = "inverse", destructive = false)
    Restore {
        scene: SceneId,
        snapshot: SceneSnapshot,
        clips: Vec<SceneClip>,
    } => {};

    /// Rename a scene. The name is trimmed, at most 200 characters; an empty name clears
    /// it. Undo: rename it back.
    "scene.rename" (kind = "edit", undo = "inverse", destructive = false)
    Rename { scene: SceneId, name: String } => {
        /// The name before the command.
        previous: String,
    };

    /// Move a scene to `index` in the list of scenes, 0 first; an index past the end
    /// means last. Refused with `invalid` when the scenes around that position share an
    /// order key (a merge made them equal), since none fits between them. Undo:
    /// `scene.restore_order` puts the exact previous order key back.
    "scene.move" (kind = "edit", undo = "inverse", destructive = false)
    Move { scene: SceneId, index: u32 } => {
        /// The scene's index before the command.
        previous_index: u32,
    };

    /// Put a scene back under an exact order key: the inverse of `scene.move`, which takes
    /// an index. Undo: set the key before this command back.
    "scene.restore_order" (kind = "edit", undo = "inverse", destructive = false)
    RestoreOrder { scene: SceneId, order: OrderKey } => {};
}

type Out = Outcome<Command, Output>;

fn not_found(scene: &SceneId) -> CommandError {
    CommandError::NotFound {
        entity: "scene".into(),
        id: scene.to_string(),
    }
}

fn invalid(field: &str, message: impl Into<String>) -> CommandError {
    CommandError::Invalid {
        field: field.into(),
        message: message.into(),
    }
}

fn check_room(project: &Project) -> Result<(), CommandError> {
    if project.scenes.len() >= MAX_SCENES {
        return Err(CommandError::Limit {
            entity: "scenes".into(),
            max: MAX_SCENES as u64,
        });
    }
    Ok(())
}

/// A name, trimmed, at most [`MAX_NAME_CHARS`] characters.
fn clean_name<'a>(name: &'a str, field: &str) -> Result<&'a str, CommandError> {
    let name = name.trim();
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(invalid(
            field,
            format!("a name has at most {MAX_NAME_CHARS} characters"),
        ));
    }
    Ok(name)
}

fn unchanged(output: Output) -> Out {
    Outcome {
        output,
        undo: Undo::None,
        events: vec![],
        control: vec![],
    }
}

fn edited(output: Output, inverse: Command, events: Vec<ProjectEvent>) -> Out {
    Outcome {
        output,
        undo: Undo::Inverse(Box::new(inverse)),
        events,
        control: vec![],
    }
}

fn changed(scene: &SceneId, part: ScenePart) -> Vec<ProjectEvent> {
    vec![ProjectEvent::SceneChanged {
        scene: scene.clone(),
        part,
    }]
}

fn scene_mut<'a>(state: &'a mut State, scene: &SceneId) -> Result<&'a mut Scene, CommandError> {
    state
        .project
        .scenes
        .get_mut(scene)
        .ok_or_else(|| not_found(scene))
}

/// Applies a command of this domain.
pub fn apply(command: &Command, state: &mut State, ctx: &mut Context) -> Result<Out, CommandError> {
    match command {
        Command::Create { name } => create(state, ctx, name.as_deref()),
        Command::Delete { scene } => delete(state, scene),
        Command::Restore {
            scene,
            snapshot,
            clips,
        } => restore(state, scene, snapshot, clips),
        Command::Rename { scene, name } => rename(state, scene, name),
        Command::Move { scene, index } => move_scene(state, scene, *index),
        Command::RestoreOrder { scene, order } => restore_order(state, scene, order),
    }
}

fn create(state: &mut State, ctx: &mut Context, name: Option<&str>) -> Result<Out, CommandError> {
    check_room(&state.project)?;
    let name = clean_name(name.unwrap_or_default(), "name")?.to_owned();
    let order = {
        let scenes = state.project.scenes_in_order();
        OrderKey::between(scenes.last().map(|(_, s)| &s.order), None)
            .map_err(|e| invalid("index", e.to_string()))?
    };
    let id = SceneId::generate(&mut ctx.ids);
    state
        .project
        .scenes
        .insert(id.clone(), Scene { name, order });
    Ok(edited(
        Output::Create { scene: id.clone() },
        Command::Delete { scene: id.clone() },
        vec![ProjectEvent::SceneCreated { scene: id }],
    ))
}

fn delete(state: &mut State, scene_id: &SceneId) -> Result<Out, CommandError> {
    if !state.project.scenes.contains_key(scene_id) {
        return Err(not_found(scene_id));
    }
    let mut taken = Vec::new();
    let mut events = Vec::new();
    for (track_id, track) in &mut state.project.tracks {
        let ids: Vec<ClipId> = track
            .clips
            .iter()
            .filter(|(_, c)| c.slot.as_ref() == Some(scene_id))
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            let Some(clip) = track.clips.remove(&id) else {
                continue;
            };
            let placements: Vec<ScenePlacement> = track
                .placements
                .iter()
                .filter(|(_, p)| p.clip == id)
                .map(|(pid, p)| ScenePlacement {
                    id: pid.clone(),
                    placement: p.clone(),
                })
                .collect();
            for p in &placements {
                track.placements.remove(&p.id);
            }
            events.push(ProjectEvent::ClipDeleted {
                track: track_id.clone(),
                clip: id.clone(),
                placements: placements.iter().map(|p| p.id.clone()).collect(),
            });
            taken.push(SceneClip {
                track: track_id.clone(),
                id,
                clip,
                placements,
            });
        }
    }
    let Some(scene) = state.project.scenes.remove(scene_id) else {
        return Err(not_found(scene_id));
    };
    events.push(ProjectEvent::SceneDeleted {
        scene: scene_id.clone(),
    });
    Ok(edited(
        Output::Delete {
            clips: u32::try_from(taken.len()).unwrap_or(u32::MAX),
        },
        Command::Restore {
            scene: scene_id.clone(),
            snapshot: SceneSnapshot {
                name: scene.name,
                order: scene.order,
            },
            clips: taken,
        },
        events,
    ))
}

fn restore(
    state: &mut State,
    scene_id: &SceneId,
    snap: &SceneSnapshot,
    clips: &[SceneClip],
) -> Result<Out, CommandError> {
    if state.project.contains_id(&scene_id.0) {
        return Err(invalid(
            "scene",
            format!("the identifier {scene_id} is already in use"),
        ));
    }
    check_room(&state.project)?;
    clean_name(&snap.name, "snapshot.name")?;
    for (i, c) in clips.iter().enumerate() {
        if c.clip.slot.as_ref() != Some(scene_id) {
            return Err(invalid(
                &format!("clips[{i}].clip.slot"),
                "the slot of a clip of the scene is the scene",
            ));
        }
        if !state.project.tracks.contains_key(&c.track) {
            return Err(CommandError::NotFound {
                entity: "track".into(),
                id: c.track.to_string(),
            });
        }
        if state.project.contains_id(&c.id.0)
            || c.placements
                .iter()
                .any(|p| state.project.contains_id(&p.id.0))
        {
            return Err(invalid(
                &format!("clips[{i}]"),
                "an identifier of the clip or of its placements is already in use",
            ));
        }
        if c.placements.iter().any(|p| p.placement.clip != c.id) {
            return Err(invalid(
                &format!("clips[{i}].placements"),
                "a placement of a clip places that clip",
            ));
        }
    }
    // The snapshot is data from outside: the whole project must stay valid with it
    // (identifiers unique across the project, limits, a slot taken once per track, clip
    // kinds). A refusal puts the project back as it was.
    let before = state.project.clone();
    state.project.scenes.insert(
        scene_id.clone(),
        Scene {
            name: snap.name.clone(),
            order: snap.order.clone(),
        },
    );
    let mut events = vec![ProjectEvent::SceneCreated {
        scene: scene_id.clone(),
    }];
    for c in clips {
        let Some(track) = state.project.tracks.get_mut(&c.track) else {
            continue;
        };
        track.clips.insert(c.id.clone(), c.clip.clone());
        for p in &c.placements {
            track.placements.insert(p.id.clone(), p.placement.clone());
        }
        events.push(ProjectEvent::ClipCreated {
            track: c.track.clone(),
            clip: c.id.clone(),
            placements: c.placements.iter().map(|p| p.id.clone()).collect(),
        });
    }
    let duplicated = clips.iter().enumerate().any(|(i, c)| {
        clips[..i].iter().any(|o| o.id == c.id)
            || c.placements
                .iter()
                .enumerate()
                .any(|(j, p)| c.placements[..j].iter().any(|o| o.id == p.id))
    });
    let valid = if duplicated {
        Err("an identifier appears twice".to_owned())
    } else {
        state.project.validate().map_err(|violations| {
            violations
                .first()
                .map_or_else(String::new, ToString::to_string)
        })
    };
    if let Err(first) = valid {
        state.project = before;
        return Err(invalid("snapshot", first));
    }
    Ok(edited(
        Output::Restore {},
        Command::Delete {
            scene: scene_id.clone(),
        },
        events,
    ))
}

fn rename(state: &mut State, scene_id: &SceneId, name: &str) -> Result<Out, CommandError> {
    let name = clean_name(name, "name")?;
    let scene = scene_mut(state, scene_id)?;
    let previous = scene.name.clone();
    let output = Output::Rename {
        previous: previous.clone(),
    };
    if previous == name {
        return Ok(unchanged(output));
    }
    scene.name = name.to_owned();
    Ok(edited(
        output,
        Command::Rename {
            scene: scene_id.clone(),
            name: previous,
        },
        changed(scene_id, ScenePart::Name),
    ))
}

fn move_scene(state: &mut State, scene_id: &SceneId, index: u32) -> Result<Out, CommandError> {
    let order = state.project.scenes_in_order();
    let previous_index = order
        .iter()
        .position(|(id, _)| *id == scene_id)
        .ok_or_else(|| not_found(scene_id))?;
    let output = Output::Move {
        previous_index: u32::try_from(previous_index).unwrap_or(u32::MAX),
    };
    let rest: Vec<(&SceneId, &OrderKey)> = order
        .iter()
        .filter(|(id, _)| *id != scene_id)
        .map(|(id, s)| (*id, &s.order))
        .collect();
    let others: Vec<&OrderKey> = rest.iter().map(|(_, k)| *k).collect();
    let index = usize::try_from(index)
        .unwrap_or(usize::MAX)
        .min(others.len());
    if index == previous_index {
        return Ok(unchanged(output));
    }
    let key = OrderKey::for_index(&others, index).map_err(|e| invalid("index", e.to_string()))?;
    // Scenes that share a key (two branches made the same one) leave no key between them:
    // refuse rather than report a move that does not happen.
    let lands = rest
        .iter()
        .filter(|(id, k)| (*k, *id) < (&key, scene_id))
        .count();
    if lands != index {
        return Err(invalid(
            "index",
            "the scenes around that position share an order key, so none fits between them",
        ));
    }
    let scene = scene_mut(state, scene_id)?;
    let before = std::mem::replace(&mut scene.order, key);
    Ok(edited(
        output,
        Command::RestoreOrder {
            scene: scene_id.clone(),
            order: before,
        },
        changed(scene_id, ScenePart::Order),
    ))
}

fn restore_order(
    state: &mut State,
    scene_id: &SceneId,
    order: &OrderKey,
) -> Result<Out, CommandError> {
    let scene = scene_mut(state, scene_id)?;
    let output = Output::RestoreOrder {};
    if scene.order == *order {
        return Ok(unchanged(output));
    }
    let before = std::mem::replace(&mut scene.order, order.clone());
    Ok(edited(
        output,
        Command::RestoreOrder {
            scene: scene_id.clone(),
            order: before,
        },
        changed(scene_id, ScenePart::Order),
    ))
}
