//! Repeatable random draws for the MIDI effects (#691): a note's draw is a hash of the device's
//! seed, the note's **place** and its key, so the same note at the same place draws the same on
//! every play, from wherever the play starts, in the export, natively and in WebAssembly.
//!
//! # The place of a note
//!
//! While the transport plays, a note's place is its position in the song, in whole ticks
//! ([`Place::Tick`], from [`song_tick`]). The engine plays a clip's note at tick `T` on the
//! frame its fractional position falls in (the floor, `transport::Segment::offset_of`), so the
//! playhead at that frame, `p` (the block's `position_ticks` plus the offset at the tempo), lies
//! within one frame's ticks `f` below `T`: the place is `round(p + f / 2)`, which is `T` while a
//! frame is shorter than a tick (above 16 kHz at any tempo up to 999 BPM; 44.1 and 48 kHz
//! always), with a margin of `(1 − f) / 2` tick against rounding, whatever the cut of the
//! blocks. A live note played meanwhile takes the playhead's place and shifts no other note's
//! draw. A loop plays the same draw on every pass: the engine counts no passes (the lead's
//! decision of 2026-11-06).
//!
//! While the transport is stopped, a note has no place in the song: the device counts the
//! note-ons it receives stopped ([`Place::Stopped`]) and restarts the count at its reset.
//!
//! # The numbers
//!
//! [`Draw::unit`] gives, per salt (a voice, a second number of the same note), a number in
//! `[0, 1)`: the top 53 bits of a splitmix64 hash over `2^53`, integer arithmetic then one `f64`
//! division, the same bits on both targets.
//!
//! Everything here runs on the audio thread: no allocation, no panic, plain arithmetic.

use crate::device::BlockCtx;

/// Where a received note-on is, for its draw (module doc).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    /// Its position in the song, in whole ticks, while the transport plays.
    Tick(i64),
    /// Its count among the note-ons received while the transport is stopped.
    Stopped(u64),
}

impl Place {
    /// The place of a note-on at frame `at` of the block of `ctx`: its tick while the transport
    /// plays, else `stopped` (the device's count of the note-ons received stopped).
    pub fn of(ctx: &BlockCtx, at: usize, stopped: u64) -> Place {
        if ctx.transport.playing {
            Place::Tick(song_tick(
                ctx.transport.position_ticks,
                at,
                ticks_per_frame(ctx),
            ))
        } else {
            Place::Stopped(stopped)
        }
    }
}

/// The ticks of one frame at the block's tempo (0 when the rate is not usable).
pub fn ticks_per_frame(ctx: &BlockCtx) -> f64 {
    let rate = f64::from(ctx.sample_rate);
    if rate.is_finite() && rate > 0.0 {
        ctx.transport.tempo.ticks_per_second() / rate
    } else {
        0.0
    }
}

/// The whole tick a note at frame `at` of a block starting at `position` (in ticks) is placed
/// at: the middle of the frame's ticks, rounded (module doc).
pub fn song_tick(position: f64, at: usize, ticks_per_frame: f64) -> i64 {
    let tick = (position + (at as f64 + 0.5) * ticks_per_frame).round();
    if tick.is_finite() {
        // Saturating: a position past the range of `i64` stays at its end.
        tick as i64
    } else {
        0
    }
}

/// The draw of one received note-on: a hash of the seed, its place and its key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Draw(u64);

impl Draw {
    pub fn new(seed: u64, place: Place, key: u8) -> Self {
        let (domain, at) = match place {
            Place::Tick(t) => (0u64, t as u64),
            Place::Stopped(n) => (1u64, n),
        };
        let h = splitmix64(seed ^ (domain << 32));
        let h = splitmix64(h ^ at);
        Draw(splitmix64(h ^ u64::from(key)))
    }

    /// The `salt`-th number in `[0, 1)` of this draw.
    pub fn unit(self, salt: u64) -> f64 {
        let h = splitmix64(self.0 ^ salt.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        (h >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Whether the `salt`-th number wins at `chance` (0 to 1): always at 1 or more, never at 0
    /// or less (nor for a chance that is not a number).
    pub fn chance(self, salt: u64, chance: f64) -> bool {
        if chance >= 1.0 {
            return true;
        }
        if chance.is_nan() || chance <= 0.0 {
            return false;
        }
        self.unit(salt) < chance
    }

    /// A choice under `choices` (0-based; 0 when `choices` is 0) from the `salt`-th number.
    pub fn pick(self, salt: u64, choices: u32) -> u32 {
        let i = (self.unit(salt) * f64::from(choices)) as u32;
        i.min(choices.saturating_sub(1))
    }
}

/// The splitmix64 mix of `z`.
pub fn splitmix64(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}
