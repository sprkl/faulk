//! Moving a note along the project's scale, shared by the MIDI effects that work "in the scale"
//! (#788). Pure integer arithmetic: no allocation, no panic.

use crate::device::PitchSet;

/// The scale's note `steps` degrees from `key`'s degree (the scale's note at or below it, or the
/// last one of the octave below when none is), as a MIDI key (may fall outside 0..=127);
/// `key + steps` when `scale` is empty.
pub fn scale_step(key: u8, steps: i32, scale: PitchSet) -> i32 {
    let mut classes = [0u8; 12];
    let mut m = 0usize;
    for pc in 0..12u8 {
        if scale.contains(pc) {
            classes[m] = pc;
            m += 1;
        }
    }
    if m == 0 {
        return i32::from(key) + steps;
    }
    let key = i32::from(key);
    let mut octave = key.div_euclid(12);
    let pc = key.rem_euclid(12);
    let index = match classes[..m].iter().rposition(|&c| i32::from(c) <= pc) {
        Some(i) => i,
        None => {
            octave -= 1;
            m - 1
        }
    };
    let total = index as i32 + steps;
    let m = m as i32;
    (octave + total.div_euclid(m)) * 12 + i32::from(classes[total.rem_euclid(m) as usize])
}

/// `key` moved by `degrees` degrees of `scale`, keeping its distance above the scale's note at
/// or below it (a key outside the scale keeps its offset), as a MIDI key (may fall outside
/// 0..=127).
pub fn degree_shift(key: u8, degrees: i32, scale: PitchSet) -> i32 {
    let offset = i32::from(key) - scale_step(key, 0, scale);
    scale_step(key, degrees, scale) + offset
}

#[cfg(test)]
mod tests {
    use super::*;

    /// C major.
    const C_MAJOR: PitchSet = PitchSet::from_bits(0b1010_1011_0101);

    #[test]
    fn degrees_move_along_the_scale_and_an_empty_scale_is_semitones() {
        assert_eq!(scale_step(60, 2, C_MAJOR), 64);
        assert_eq!(scale_step(61, 1, C_MAJOR), 62, "from the note at or below");
        assert_eq!(scale_step(60, -1, C_MAJOR), 59);
        assert_eq!(scale_step(60, 3, PitchSet::EMPTY), 63);
        assert_eq!(degree_shift(60, 2, C_MAJOR), 64);
        assert_eq!(
            degree_shift(61, 2, C_MAJOR),
            65,
            "a key off the scale keeps its offset"
        );
        assert_eq!(degree_shift(61, 2, PitchSet::EMPTY), 63);
    }
}
