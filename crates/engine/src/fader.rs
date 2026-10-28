//! The fader's law (#389): the one mapping between the **travel** of a volume fader (a
//! position from 0, the bottom, to 1, the top) and the gain in dB it stands for.
//!
//! The mixer's fader, the vertical axis of a volume lane in the timeline and the engine's
//! reading of a volume lane all follow this table, so that a straight line drawn on a lane is
//! the curve the engine plays: a line from 0 dB to -96 dB over eight bars goes down like a
//! fader pulled at constant speed. The table is exported by `crates/protocol` in the schema
//! bundle (`fader_taper`), from which the web app builds its own fader (`FADER_TAPER`); no
//! other copy exists.
//!
//! The taper is piecewise linear in dB through [`TAPER`]: linear in dB over the whole range
//! would spend two thirds of the travel below -60 dB, where nothing is mixed. The bottom
//! (position 0, [`MIN_DB`]) is silence: [`gain_at`] answers an amplitude of exactly 0 there.
//!
//! Nothing here allocates or panics; every input is clamped, NaN is the bottom.

/// The points of the taper: `(position, dB)`, positions strictly increasing from 0 to 1.
pub const TAPER: [(f64, f64); 6] = [
    (0.0, MIN_DB),
    (0.08, -60.0),
    (0.28, -30.0),
    (0.5, -12.0),
    (0.78, 0.0),
    (1.0, MAX_DB),
];

/// The gain of the bottom of the fader: silence.
pub const MIN_DB: f64 = -96.0;

/// The gain of the top of the fader.
pub const MAX_DB: f64 = 6.0;

/// The position of a gain in dB (clamped to the table, NaN is the bottom).
pub fn position_of_db(db: f64) -> f64 {
    if db.is_nan() || db <= MIN_DB {
        return 0.0;
    }
    if db >= MAX_DB {
        return 1.0;
    }
    let mut i = 1;
    while i < TAPER.len() - 1 && db > TAPER[i].1 {
        i += 1;
    }
    let ((p0, d0), (p1, d1)) = (TAPER[i - 1], TAPER[i]);
    p0 + (db - d0) / (d1 - d0) * (p1 - p0)
}

/// The gain in dB at a position (clamped to 0..1, NaN is the bottom).
pub fn db_at(position: f64) -> f64 {
    if position.is_nan() || position <= 0.0 {
        return MIN_DB;
    }
    if position >= 1.0 {
        return MAX_DB;
    }
    let mut i = 1;
    while i < TAPER.len() - 1 && position > TAPER[i].0 {
        i += 1;
    }
    let ((p0, d0), (p1, d1)) = (TAPER[i - 1], TAPER[i]);
    d0 + (position - p0) / (p1 - p0) * (d1 - d0)
}

/// The linear gain (amplitude) at a position: 0 at the bottom, silence.
pub fn gain_at(position: f64) -> f32 {
    if position.is_nan() || position <= 0.0 {
        return 0.0;
    }
    10f64.powf(db_at(position) / 20.0) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_taper_is_the_documented_table() {
        let want = [
            (0.0, -96.0),
            (0.08, -60.0),
            (0.28, -30.0),
            (0.5, -12.0),
            (0.78, 0.0),
            (1.0, 6.0),
        ];
        assert_eq!(TAPER, want);
        assert!(TAPER.windows(2).all(|w| w[0].0 < w[1].0 && w[0].1 < w[1].1));
    }

    #[test]
    fn positions_and_gains_round_trip_through_every_segment() {
        for db in [
            -96.0, -80.0, -60.0, -45.0, -30.0, -20.0, -12.0, -6.0, 0.0, 3.0, 6.0,
        ] {
            let back = db_at(position_of_db(db));
            assert!((back - db).abs() < 1e-9, "{db} came back as {back}");
        }
        assert_eq!(position_of_db(0.0), 0.78);
        assert_eq!(db_at(0.5), -12.0);
    }

    #[test]
    fn the_bottom_is_silence_and_the_input_is_clamped() {
        assert_eq!(gain_at(0.0), 0.0);
        assert_eq!(gain_at(-1.0), 0.0);
        assert_eq!(gain_at(f64::NAN), 0.0);
        assert_eq!(position_of_db(f64::NAN), 0.0);
        assert_eq!(position_of_db(f64::NEG_INFINITY), 0.0);
        assert_eq!(position_of_db(1_000.0), 1.0);
        assert_eq!(db_at(7.0), MAX_DB);
        assert_eq!(gain_at(f64::INFINITY), gain_at(1.0));
        assert_eq!(gain_at(0.78), 1.0);
        assert!(gain_at(1e-9) > 0.0);
    }
}
