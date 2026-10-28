//! The id → engine slot table (PR-3, `docs/specs/02-decoupage-mvp.md` § 2.3).
//!
//! The engine never sees a stable identifier: every structure a `ControlMsg` names is a
//! **slot**, a small index the project assigns. This module keeps the correspondence for
//! one engine connection: which slot each track, device, clip, placement, scene and lane
//! of the project has in the engine, and what each track's engine entry holds (so that
//! deleting a track removes what the engine has of it, though the project no longer does).
//!
//! - **Reuse**: [`SlotTable::assign`] gives the lowest free slot; a freed slot is the next
//!   one given. Slots stay dense, so the engine's slot arrays stay as small as the project.
//! - **Growth**: a table has no capacity of its own; a new id beyond every used slot takes
//!   the next one, and the engine grows its own table when a message names a slot beyond
//!   it (`engine::msg`, growth). The only bound is the engine's limit for that kind of
//!   slot, passed to `assign`.
//! - **Canonical**: trailing free slots are trimmed, so a table compares equal to the one
//!   it was before an assign and the matching release.
//!
//! It is rebuilt from the project by every snapshot (an engine reset), and changed by each
//! command's engine messages (`commands::engine_msgs`).

use crate::ids::{ClipId, DeviceId, LaneId, PlacementId, SceneId, TrackId};
use crate::time::Tick;
use std::collections::{BTreeMap, BTreeSet};

/// Ids of one kind and their slots.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlotTable<K: Ord + Clone> {
    /// Slot → id; `None` for a free slot. Never ends with `None`.
    by_slot: Vec<Option<K>>,
    by_id: BTreeMap<K, u32>,
    /// The free slots below `by_slot.len()`, so an assign never scans the table.
    free: BTreeSet<u32>,
}

impl<K: Ord + Clone> Default for SlotTable<K> {
    fn default() -> Self {
        Self {
            by_slot: Vec::new(),
            by_id: BTreeMap::new(),
            free: BTreeSet::new(),
        }
    }
}

impl<K: Ord + Clone> SlotTable<K> {
    /// The slot of `id`, if it has one.
    pub fn get(&self, id: &K) -> Option<u32> {
        self.by_id.get(id).copied()
    }

    /// The id in `slot`, if any.
    pub fn id_at(&self, slot: u32) -> Option<&K> {
        self.by_slot.get(slot as usize).and_then(Option::as_ref)
    }

    /// The slot of `id`: the one it has, or else the lowest free slot, or else the next
    /// one. `None` when every slot below `limit` is taken (nothing changes then).
    pub fn assign(&mut self, id: &K, limit: u32) -> Option<u32> {
        if let Some(slot) = self.get(id) {
            return Some(slot);
        }
        let slot = match self.free.first() {
            Some(free) => *free,
            None => u32::try_from(self.by_slot.len()).ok()?,
        };
        if slot >= limit {
            return None;
        }
        self.put(id, slot);
        Some(slot)
    }

    /// Gives `id` this exact slot (a device's place in its chain). `None` when the slot is
    /// at or beyond `limit`, taken by another id, or `id` already has another slot.
    pub fn insert_at(&mut self, id: &K, slot: u32, limit: u32) -> Option<u32> {
        if slot >= limit {
            return None;
        }
        match (self.get(id), self.id_at(slot)) {
            (Some(s), _) if s == slot => Some(slot),
            (None, None) => {
                self.put(id, slot);
                Some(slot)
            }
            _ => None,
        }
    }

    fn put(&mut self, id: &K, slot: u32) {
        let at = slot as usize;
        if at >= self.by_slot.len() {
            self.free.extend(self.by_slot.len() as u32..slot);
            self.by_slot.resize(at + 1, None);
        }
        self.free.remove(&slot);
        self.by_slot[at] = Some(id.clone());
        self.by_id.insert(id.clone(), slot);
    }

    /// Frees the slot of `id`; returns it.
    pub fn release(&mut self, id: &K) -> Option<u32> {
        let slot = self.by_id.remove(id)?;
        if let Some(entry) = self.by_slot.get_mut(slot as usize) {
            *entry = None;
            self.free.insert(slot);
        }
        while self.by_slot.last().is_some_and(Option::is_none) {
            self.by_slot.pop();
            self.free.remove(&(self.by_slot.len() as u32));
        }
        Some(slot)
    }

    /// How many ids have a slot.
    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    /// One past the highest slot in use: the size the engine's table needs.
    pub fn span(&self) -> u32 {
        self.by_slot.len() as u32
    }

    /// The ids and their slots, by slot.
    pub fn iter(&self) -> impl Iterator<Item = (u32, &K)> {
        self.by_slot
            .iter()
            .enumerate()
            .filter_map(|(slot, id)| id.as_ref().map(|id| (slot as u32, id)))
    }
}

/// What the engine holds of one track, besides its slot.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrackSlots {
    /// Its devices sent to the engine; a device's slot is its place in the chain, so a
    /// device the engine cannot build leaves a hole.
    pub devices: SlotTable<DeviceId>,
    /// Its clips sent to the engine, with the scene of the launch-grid slot each sits in.
    pub clips: BTreeMap<ClipId, Option<SceneId>>,
    /// Its placements in the engine's arrangement of the track.
    pub placements: BTreeSet<PlacementId>,
    /// Its automation lanes sent to the engine.
    pub lanes: BTreeSet<LaneId>,
}

/// The project's view of one engine: the slots of everything sent to it, and the
/// playback orders it was given. Kept per engine connection, rebuilt by every snapshot.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EngineSlots {
    pub tracks: SlotTable<TrackId>,
    /// The master's devices sent to the engine (#364); the slot of a device is its place in
    /// the master's chain, as for a track.
    pub master_devices: SlotTable<DeviceId>,
    /// What each track with a slot holds in the engine.
    pub track: BTreeMap<TrackId, TrackSlots>,
    pub clips: SlotTable<ClipId>,
    pub placements: SlotTable<PlacementId>,
    pub scenes: SlotTable<SceneId>,
    pub lanes: SlotTable<LaneId>,
    /// Where `transport.play` starts: the marker, or the position a `transport.pause` left
    /// the playhead on, 0 at first.
    pub cue: Tick,
    /// The marker (#360): the last position set by `transport.seek` (a ruler click), 0 at
    /// first. `transport.stop` brings the playhead back to it, a pause and a resume never
    /// move it.
    pub marker: Tick,
    /// Whether the last playback order was play.
    pub playing: bool,
    /// The live input notes the engine holds, as `(note, track)`: where each
    /// `live.input_note_on` went, so its note off reaches the same tracks even when the
    /// selection or the arming changed meanwhile (RE-1). The engine forgets them with a
    /// `Reset`, as this table is rebuilt by it.
    pub held: BTreeSet<(u8, TrackId)>,
    /// Entities left out because the engine's table of their kind was full: the project
    /// has more clips, placements or lanes than the engine takes (the project limits
    /// those per track only).
    pub overflow: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn freed_slots_are_reused_lowest_first() {
        let mut t = SlotTable::<u32>::default();
        for id in 0..5 {
            assert_eq!(t.assign(&id, 100), Some(id));
        }
        assert_eq!(t.release(&3), Some(3));
        assert_eq!(t.release(&1), Some(1));
        assert_eq!(t.assign(&10, 100), Some(1));
        assert_eq!(t.assign(&11, 100), Some(3));
        assert_eq!(t.assign(&12, 100), Some(5));
        assert_eq!(t.assign(&10, 100), Some(1), "an id keeps its slot");
        assert_eq!(t.id_at(3), Some(&11));
    }

    #[test]
    fn the_table_grows_and_stops_at_the_limit() {
        let mut t = SlotTable::<u32>::default();
        for id in 0..1000 {
            assert_eq!(t.assign(&id, 1000), Some(id));
        }
        assert_eq!(t.span(), 1000);
        assert_eq!(t.assign(&5000, 1000), None, "full");
        assert_eq!(t.len(), 1000);
        t.release(&500);
        assert_eq!(t.assign(&5000, 1000), Some(500));
    }

    #[test]
    fn an_assign_then_its_release_leaves_the_table_as_it_was() {
        let mut t = SlotTable::<u32>::default();
        t.assign(&1, 10);
        t.assign(&2, 10);
        t.release(&1);
        let before = t.clone();
        t.assign(&7, 10);
        t.assign(&8, 10);
        t.release(&8);
        t.release(&7);
        assert_eq!(t, before);
        t.release(&2);
        assert_eq!(t, SlotTable::default(), "trailing free slots are trimmed");
        assert_eq!(t.span(), 0);
    }

    #[test]
    fn insert_at_keeps_holes_and_refuses_a_taken_slot() {
        let mut t = SlotTable::<u32>::default();
        assert_eq!(t.insert_at(&1, 0, 4), Some(0));
        assert_eq!(t.insert_at(&3, 2, 4), Some(2));
        assert_eq!(t.iter().collect::<Vec<_>>(), [(0, &1), (2, &3)]);
        assert_eq!(t.insert_at(&4, 2, 4), None, "taken");
        assert_eq!(t.insert_at(&1, 1, 4), None, "the id has another slot");
        assert_eq!(t.insert_at(&5, 4, 4), None, "beyond the limit");
        assert_eq!(t.insert_at(&3, 2, 4), Some(2));
        assert_eq!(t.assign(&9, 4), Some(1), "the hole is the lowest free slot");
    }
}
