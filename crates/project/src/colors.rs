//! The track colour palette and how a project hands its colours out.
//!
//! A track's colour is stored in the project from its creation, so that moving or deleting
//! other tracks never changes it. A track the document holds without a colour (a project
//! saved before colours were stored) shows the hue of its position in the web app; opening
//! such a project stores those hues ([`Project::color_uncolored_tracks`]).

use crate::model::Project;
use crate::serde_util::Rgb;

/// The sixteen track hues of the design system (DS-0.1, `track-01` to `track-16`), in the
/// order of the web palette (`TRACK_COLORS` in `web/src/lib/project/tracks.ts`, whose test
/// pins the same values).
pub const TRACK_PALETTE: [u32; 16] = [
    0xf08098, 0xf18a76, 0xea9356, 0xd99e3f, 0xc1ab3c, 0xa0b751, 0x79bf72, 0x4ac495, 0x00c5b7,
    0x00c1d5, 0x3bb9ed, 0x6bafff, 0x92a4fc, 0xb299f3, 0xcd90e0, 0xe189c5,
];

/// The `i`-th hue of the palette (wrapping).
pub fn palette_hue(i: usize) -> Rgb {
    let v = TRACK_PALETTE[i % TRACK_PALETTE.len()];
    Rgb::new((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

impl Project {
    /// The hue of the next track: the first of the palette no track stores yet, else (all
    /// sixteen in use) the first of the least used. Independent of the order of the tracks.
    pub fn next_track_color(&self) -> Rgb {
        let used = |i: usize| {
            let hue = Some(palette_hue(i));
            self.tracks.values().filter(|t| t.color == hue).count()
        };
        let counts: Vec<usize> = (0..TRACK_PALETTE.len()).map(used).collect();
        let least = counts.iter().copied().min().unwrap_or(0);
        palette_hue(counts.iter().position(|&n| n == least).unwrap_or(0))
    }

    /// Stores on each track without a colour the hue of its position (display order), the
    /// one the page showed it. Returns how many it coloured. A normalisation of what is
    /// read, not an edit (no history).
    pub fn color_uncolored_tracks(&mut self) -> usize {
        let ids: Vec<_> = self
            .tracks_in_order()
            .into_iter()
            .map(|(id, _)| id.clone())
            .collect();
        let mut colored = 0;
        for (i, id) in ids.iter().enumerate() {
            if let Some(track) = self.tracks.get_mut(id)
                && track.color.is_none()
            {
                track.color = Some(palette_hue(i));
                colored += 1;
            }
        }
        colored
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Track;
    use crate::{IdGenerator, OrderKey, TrackId, TrackKind};

    /// The web palette (`TRACK_COLORS`) in order: a drift between the two breaks the picker.
    #[test]
    fn the_palette_is_the_web_palette() {
        let web = [
            "#f08098", "#f18a76", "#ea9356", "#d99e3f", "#c1ab3c", "#a0b751", "#79bf72", "#4ac495",
            "#00c5b7", "#00c1d5", "#3bb9ed", "#6bafff", "#92a4fc", "#b299f3", "#cd90e0", "#e189c5",
        ];
        let rust: Vec<String> = (0..16).map(|i| palette_hue(i).to_string()).collect();
        assert_eq!(rust, web);
    }

    fn project_with(n: usize) -> Project {
        let mut ids = IdGenerator::for_tests();
        let mut p = Project::new(crate::ProjectId::generate(&mut ids), "P");
        let mut key = OrderKey::first();
        for i in 0..n {
            let mut t = Track::new(TrackKind::Midi, "", key.clone());
            t.color = Some(palette_hue(i % 16));
            p.tracks.insert(TrackId::generate(&mut ids), t);
            key = OrderKey::between(Some(&key), None).unwrap();
        }
        p
    }

    #[test]
    fn the_next_hue_is_the_first_free_then_the_first_least_used() {
        assert_eq!(project_with(0).next_track_color(), palette_hue(0));
        assert_eq!(project_with(5).next_track_color(), palette_hue(5));
        // All sixteen once: the first again; 17 tracks (hue 0 twice): hue 1.
        assert_eq!(project_with(16).next_track_color(), palette_hue(0));
        assert_eq!(project_with(17).next_track_color(), palette_hue(1));
        let mut gap = project_with(16);
        let id = gap.tracks.keys().nth(3).unwrap().clone();
        gap.tracks.remove(&id);
        assert_eq!(
            gap.next_track_color(),
            palette_hue(3),
            "a freed hue is reused"
        );
    }
}
