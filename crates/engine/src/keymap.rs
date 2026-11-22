//! The key changes the audio side reads (#953): a sorted table of (tick, pitch-class set), a
//! fixed size, built off the audio path (`prepare`) and swapped in whole by `apply`
//! (`RtMsg::SetKeyChanges`). The audio side only reads it, at the start of each block, with a
//! binary search over at most [`MAX_KEY_CHANGES`] entries: no allocation, no lock, no panic.
//!
//! The scale in force at a position is the set of the last change at or before it, or the
//! starting scale (`State::scale`) before the first one. An entry with the empty set is a
//! change to "no scale" (chromatic), like a project without one.

use crate::device::{PitchSet, ScaleSwitches};
use crate::msg::limits;
use crate::transport::Segment;

/// Most key changes the table holds: the project's limit, enforced before a message is sent.
pub const MAX_KEY_CHANGES: usize = limits::MAX_KEY_CHANGES as usize;

/// The key changes of the project, by tick (module doc).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyMap {
    len: usize,
    ticks: [i64; MAX_KEY_CHANGES],
    sets: [u16; MAX_KEY_CHANGES],
}

impl KeyMap {
    /// A table of these changes, `(tick, pitch-class bits)`, or `None` when they are not
    /// strictly increasing, not after tick 0, or more than [`MAX_KEY_CHANGES`]: the binary
    /// search below needs the order, so a table that does not hold it is never built.
    pub fn new(changes: &[(i64, u16)]) -> Option<Self> {
        if changes.len() > MAX_KEY_CHANGES {
            return None;
        }
        let mut map = KeyMap {
            len: changes.len(),
            ticks: [0; MAX_KEY_CHANGES],
            sets: [0; MAX_KEY_CHANGES],
        };
        let mut previous = 0i64;
        for (i, &(tick, bits)) in changes.iter().enumerate() {
            if tick <= previous {
                return None;
            }
            previous = tick;
            map.ticks[i] = tick;
            map.sets[i] = bits & PitchSet::MASK;
        }
        Some(map)
    }

    /// Changes in the table.
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The changes, in tick order.
    pub fn changes(&self) -> impl Iterator<Item = (i64, PitchSet)> + '_ {
        (0..self.len).map(|i| (self.ticks[i], PitchSet::from_bits(self.sets[i])))
    }

    /// Index of the first change after `position` (a fractional tick; NaN is before every
    /// change): the number of changes at or before it.
    fn after(&self, position: f64) -> usize {
        self.ticks[..self.len].partition_point(|&t| t as f64 <= position)
    }

    /// The scale in force at `position`: the last change at or before it, else `start`.
    pub fn scale_at(&self, start: PitchSet, position: f64) -> PitchSet {
        match self.after(position).checked_sub(1) {
            Some(i) => PitchSet::from_bits(self.sets[i]),
            None => start,
        }
    }

    /// The places of the block where the scale changes from `first` (the scale at the block's
    /// first position), in frame order: a segment that starts in another key (a loop wrap)
    /// switches at its first frame; a change inside a segment at the frame that holds its tick
    /// (`Segment::offset_of`, the floor the events use), so an event at that tick sees the new
    /// key. At most [`ScaleSwitches::CAPACITY`] per block (a loop shorter than a few
    /// milliseconds across keys passes it; the later ones are left out). No allocation.
    pub fn switches_in(
        &self,
        start: PitchSet,
        first: PitchSet,
        segments: &[Segment],
    ) -> ScaleSwitches {
        let mut out = ScaleSwitches::NONE;
        if self.is_empty() {
            return out;
        }
        let mut current = first;
        for segment in segments {
            let from = segment.from.0 as f64;
            let at_from = self.scale_at(start, from);
            if at_from != current && out.push(segment.offset as u32, at_from) {
                current = at_from;
            }
            let mut i = self.after(from);
            while i < self.len && self.ticks[i] < segment.to.0 {
                let set = PitchSet::from_bits(self.sets[i]);
                if set != current {
                    let frame = segment.offset_of(crate::time::Tick(self.ticks[i]));
                    // A switch that does not fit is left out, and the key it would have set is not the
                    // one in force for what follows: the last one that fitted is.
                    if out.push(frame as u32, set) {
                        current = set;
                    }
                }
                i += 1;
            }
        }
        out
    }
}
