//! The table of received notes that sound, shared by the MIDI effects (#788): a fixed table
//! built in the factory, whose entries carry the effect's own value `T` (what to end the note
//! with). A generation counter empties it in O(1): an entry of an older generation holds
//! nothing. No allocation after construction.

use crate::msg::{MsgError, try_box};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Entry<T> {
    /// The entry holds a note when this is the table's generation (never 0, the generation of
    /// an entry never used or released).
    generation: u32,
    /// The received note's voice id.
    voice: u32,
    value: T,
}

/// `N` received notes held at once, each with a `T`.
pub struct HeldTable<T: Copy + Default, const N: usize> {
    entries: Box<[Entry<T>; N]>,
    generation: u32,
}

impl<T: Copy + Default, const N: usize> HeldTable<T, N> {
    /// The bytes the table takes (for a device's `state_bytes`).
    pub const BYTES: usize = size_of::<[Entry<T>; N]>();

    /// An empty table (a refused allocation is a typed error).
    pub fn new() -> Result<Self, MsgError> {
        Ok(Self {
            entries: try_box([Entry::<T>::default(); N])?,
            generation: 1,
        })
    }

    /// The entry holding `voice`, if any.
    pub fn find(&self, voice: u32) -> Option<usize> {
        self.entries
            .iter()
            .position(|e| e.generation == self.generation && e.voice == voice)
    }

    /// The first free entry, if any.
    pub fn free(&self) -> Option<usize> {
        self.entries
            .iter()
            .position(|e| e.generation != self.generation)
    }

    /// The first free entry and the entry holding `voice`, in one pass.
    pub fn free_and_find(&self, voice: u32) -> (Option<usize>, Option<usize>) {
        let (mut free, mut same) = (None, None);
        for (i, e) in self.entries.iter().enumerate() {
            if e.generation != self.generation {
                free = free.or(Some(i));
            } else if e.voice == voice {
                same = Some(i);
                break;
            }
        }
        (free, same)
    }

    /// Puts `value` for `voice` in entry `index` (a free one) and gives it back.
    pub fn insert(&mut self, index: usize, voice: u32, value: T) -> Option<&mut T> {
        let generation = self.generation;
        let e = self.entries.get_mut(index)?;
        *e = Entry {
            generation,
            voice,
            value,
        };
        Some(&mut e.value)
    }

    /// The value of entry `index` when it holds a note.
    pub fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        let generation = self.generation;
        self.entries
            .get_mut(index)
            .filter(|e| e.generation == generation)
            .map(|e| &mut e.value)
    }

    /// Frees entry `index`.
    pub fn release(&mut self, index: usize) {
        if let Some(e) = self.entries.get_mut(index) {
            e.generation = 0;
        }
    }

    /// The held entries and their values, in table order.
    pub fn iter(&self) -> impl Iterator<Item = (usize, &T)> {
        let generation = self.generation;
        self.entries
            .iter()
            .enumerate()
            .filter(move |(_, e)| e.generation == generation)
            .map(|(i, e)| (i, &e.value))
    }

    /// Empties the table in O(1).
    pub fn clear(&mut self) {
        self.generation = match self.generation.wrapping_add(1) {
            0 => 1,
            g => g,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_table_holds_finds_releases_and_empties_in_one_step() {
        let mut t = HeldTable::<u8, 2>::new().unwrap();
        assert_eq!(t.free(), Some(0));
        *t.insert(0, 7, 60).unwrap() += 1;
        t.insert(1, 8, 62).unwrap();
        assert_eq!(t.free(), None, "full");
        assert_eq!(t.find(7), Some(0));
        assert_eq!(t.free_and_find(8), (None, Some(1)));
        assert_eq!(
            t.iter().map(|(i, v)| (i, *v)).collect::<Vec<_>>(),
            [(0, 61), (1, 62)]
        );
        t.release(0);
        assert_eq!(
            (t.find(7), t.free(), t.get_mut(0).is_none()),
            (None, Some(0), true)
        );
        t.clear();
        assert_eq!((t.find(8), t.iter().count()), (None, 0));
        assert!(t.insert(5, 1, 1).is_none(), "out of range");
    }

    #[test]
    fn the_generation_wraps_without_resurrecting_an_entry() {
        let mut t = HeldTable::<u8, 1>::new().unwrap();
        t.insert(0, 1, 1).unwrap();
        t.generation = u32::MAX;
        t.entries[0].generation = u32::MAX;
        t.clear();
        assert_eq!(t.generation, 1);
        assert_eq!(t.find(1), None);
    }
}
