//! The detection sources of the devices (#952, a sidechain), as a graph of tracks: a track
//! **listens to** another when one of its devices names it ([`crate::Device::sidechain`]).
//! The engine renders a track after the tracks it listens to, so these edges never loop
//! ([`crate::Project::validate`]).

use std::collections::{BTreeMap, BTreeSet};

use crate::ids::{DeviceId, TrackId};
use crate::model::Project;

/// The tracks `track` listens to, through its devices (each once).
pub fn sources_of<'p>(project: &'p Project, track: &TrackId) -> BTreeSet<&'p TrackId> {
    project
        .tracks
        .get(track)
        .map(|t| {
            t.devices
                .values()
                .filter_map(|d| d.sidechain.as_ref())
                .collect()
        })
        .unwrap_or_default()
}

/// The devices of other tracks that listen to `track`, with their track.
pub fn listeners_of(project: &Project, track: &TrackId) -> Vec<(TrackId, DeviceId)> {
    project
        .tracks
        .iter()
        .filter(|(id, _)| *id != track)
        .flat_map(|(id, t)| {
            t.devices
                .iter()
                .filter(|(_, d)| d.sidechain.as_ref() == Some(track))
                .map(move |(device, _)| (id.clone(), device.clone()))
        })
        .collect()
}

/// The tracks some device listens to, each once.
pub fn sources(project: &Project) -> BTreeSet<&TrackId> {
    project
        .tracks
        .values()
        .flat_map(|t| t.devices.values())
        .filter_map(|d| d.sidechain.as_ref())
        .collect()
}

/// Whether `from` listens to `to`, directly or through the tracks it listens to (a track
/// reaches itself).
pub fn reaches(project: &Project, from: &TrackId, to: &TrackId) -> bool {
    let mut seen = BTreeSet::new();
    let mut stack = vec![from];
    while let Some(t) = stack.pop() {
        if t == to {
            return true;
        }
        if seen.insert(t) {
            stack.extend(sources_of(project, t));
        }
    }
    false
}

/// A track on a loop of the edges, if there is one.
pub fn cycle(project: &Project) -> Option<TrackId> {
    // 1: on the walk's stack, 2: done.
    let mut mark: BTreeMap<&TrackId, u8> = BTreeMap::new();
    for root in project.tracks.keys() {
        if mark.contains_key(root) {
            continue;
        }
        let mut stack: Vec<(&TrackId, Vec<&TrackId>)> =
            vec![(root, sources_of(project, root).into_iter().collect())];
        mark.insert(root, 1);
        while let Some((t, next)) = stack.last_mut() {
            let t = *t;
            match next.pop() {
                Some(s) => match mark.get(s) {
                    Some(1) => return Some(s.clone()),
                    Some(_) => {}
                    None => {
                        mark.insert(s, 1);
                        let more = sources_of(project, s).into_iter().collect();
                        stack.push((s, more));
                    }
                },
                None => {
                    mark.insert(t, 2);
                    stack.pop();
                }
            }
        }
    }
    None
}
