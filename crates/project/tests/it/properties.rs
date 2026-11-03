//! Property tests: generated valid projects pass `validate()` and survive a serde round
//! trip unchanged, byte for byte on the second write.

use project::*;
use proptest::prelude::*;
use std::collections::{BTreeMap, BTreeSet};

use crate::common;
use common::*;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn generated_projects_are_valid_and_round_trip(
        seed in any::<u128>(),
        name in name(),
        tracks in prop::collection::vec(track_spec(), 0..6),
        scenes in 0usize..5,
    ) {
        let project = build(seed, name, tracks, scenes);
        prop_assert_eq!(project.validate(), Ok(()));
        let json = serde_json::to_string(&project).unwrap();
        let back: Project = serde_json::from_str(&json).unwrap();
        prop_assert_eq!(&back, &project);
        prop_assert_eq!(serde_json::to_string(&back).unwrap(), json);
        prop_assert_eq!(back.validate(), Ok(()));
    }

    #[test]
    fn sessions_round_trip(
        solo in prop::collection::btree_set(any::<u64>(), 0..10),
        input in prop::option::of(name()),
    ) {
        let mut ids = IdGenerator::new(1, 1);
        let all: Vec<TrackId> = solo.iter().map(|_| TrackId::generate(&mut ids)).collect();
        let session = SessionState {
            selected_track: all.first().cloned(),
            armed: all.iter().step_by(2).cloned().collect(),
            monitor: all.iter().skip(1).step_by(2).cloned().collect(),
            solo: all.into_iter().collect(),
            midi_input: input,
            record: Default::default(),
        };
        let json = serde_json::to_string(&session).unwrap();
        prop_assert_eq!(serde_json::from_str::<SessionState>(&json).unwrap(), session);
    }
}

proptest! {
    /// The slot table against a naive model: every assign gives the lowest slot no live id
    /// holds, ids keep their slots, a full table refuses, and releasing everything leaves
    /// the empty table.
    #[test]
    fn slot_table_gives_the_lowest_free_slot(ops in prop::collection::vec((any::<bool>(), 0u8..40), 0..300)) {
        const LIMIT: u32 = 24;
        let mut table = SlotTable::<u8>::default();
        let mut model: BTreeMap<u8, u32> = BTreeMap::new();
        for (assign, id) in ops {
            if assign {
                let expected = model.get(&id).copied().or_else(|| {
                    let used: BTreeSet<u32> = model.values().copied().collect();
                    (0..LIMIT).find(|s| !used.contains(s))
                });
                prop_assert_eq!(table.assign(&id, LIMIT), expected);
                if let Some(slot) = expected {
                    model.insert(id, slot);
                }
            } else {
                prop_assert_eq!(table.release(&id), model.remove(&id));
            }
            prop_assert_eq!(table.len(), model.len());
            prop_assert_eq!(table.span(), model.values().max().map_or(0, |m| m + 1));
            for (id, slot) in &model {
                prop_assert_eq!(table.get(id), Some(*slot));
                prop_assert_eq!(table.id_at(*slot), Some(id));
            }
        }
        for id in model.keys() {
            table.release(id);
        }
        prop_assert_eq!(table, SlotTable::default());
    }
}
