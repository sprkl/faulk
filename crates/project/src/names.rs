//! Default names: the name a clip gets when it is created ("Bass 3": its track's name and
//! a number), the rule a copied track's name follows, and the load-time pass that names the
//! clips of a project saved before clips were named.
//!
//! A copied clip keeps its source's name (#473): the copy plays the same part, and its name
//! says so. A copied track takes the next name ([`copy_name`]); the web app has the same rule
//! for tracks (`copyName` in `web/src/lib/project/tracks.ts`), kept in step by hand.

use crate::ids::{ClipId, TrackId};
use crate::limits::MAX_NAME_CHARS;
use crate::model::{Project, Track};
use std::collections::BTreeSet;

/// The base of a default clip name when the track has no usable name.
pub const CLIP_PREFIX: &str = "Clip";

/// The base of the default names of a track's clips: the track's name, trimmed and
/// shortened so that the base, a space and a number of ten digits fit [`MAX_NAME_CHARS`].
fn clip_base(track: &Track) -> String {
    let name = track.name.trim();
    let base = if name.is_empty() { CLIP_PREFIX } else { name };
    base.chars().take(MAX_NAME_CHARS - 11).collect()
}

/// The number of a default clip name of `base` ("Bass 3" is 3 for "Bass"), `None` for any
/// other name. Only the canonical spelling counts: "Bass 03" is the user's own name.
fn clip_number(base: &str, name: &str) -> Option<u32> {
    let n: u32 = name.strip_prefix(base)?.strip_prefix(' ')?.parse().ok()?;
    (n > 0 && format!("{base} {n}") == name).then_some(n)
}

/// The name of the copy of something called `label`: its trailing number plus one ("Clip 2"
/// gives "Clip 3", "Synth" gives "Synth 2"; a number of more than two digits is part of the
/// name), the first one that is not in `taken`. A name has at most [`MAX_NAME_CHARS`]
/// characters: the base is shortened, never the number.
pub fn copy_name<'a>(label: &str, taken: impl IntoIterator<Item = &'a str>) -> String {
    let taken: BTreeSet<&str> = taken.into_iter().collect();
    let (base, first) = split_number(label);
    for n in first.. {
        let whole = format!("{base} {n}");
        let candidate = if whole.chars().count() <= MAX_NAME_CHARS {
            whole
        } else {
            let keep = MAX_NAME_CHARS.saturating_sub(10);
            format!("{} {n}", base.chars().take(keep).collect::<String>())
        };
        if !taken.contains(candidate.as_str()) {
            return candidate;
        }
    }
    unreachable!("a number is eventually free")
}

/// `(base, number to try first)`: "Clip 2" is `("Clip", 3)`, "Synth" is `("Synth", 2)`.
fn split_number(label: &str) -> (&str, u32) {
    let digits = label.len() - label.trim_end_matches(|c: char| c.is_ascii_digit()).len();
    if (1..=2).contains(&digits) {
        let (head, tail) = label.split_at(label.len() - digits);
        let base = head.trim_end();
        // The separator must exist and the base must have text in it.
        if base.len() < head.len() && !base.is_empty() {
            return (base, tail.parse::<u32>().map_or(2, |n| n + 1));
        }
    }
    (label, 2)
}

impl Project {
    /// The name of the next new clip of `track`: the track's name and the first number no
    /// clip of the track has with it ("Bass 1", "Bass 2"; a gap left by a deleted clip is
    /// filled). "Clip n" when the track does not exist.
    pub fn next_clip_name(&self, track: &TrackId) -> String {
        self.next_clip_name_beside(track, &[])
    }

    /// [`Project::next_clip_name`] when `also` are the names of clips a command is adding to
    /// the track and the project does not hold yet.
    pub fn next_clip_name_beside(&self, track: &TrackId, also: &[String]) -> String {
        let Some(t) = self.tracks.get(track) else {
            return format!("{CLIP_PREFIX} 1");
        };
        let base = clip_base(t);
        let used: BTreeSet<u32> = t
            .clips
            .values()
            .map(|c| c.name.as_str())
            .chain(also.iter().map(String::as_str))
            .filter_map(|name| clip_number(&base, name))
            .collect();
        let n = (1u32..).find(|n| !used.contains(n)).unwrap_or(u32::MAX);
        format!("{base} {n}")
    }

    /// Whether `name` is a default name of a clip of `track` ("Bass 3" in the track "Bass").
    pub fn is_default_clip_name(&self, track: &TrackId, name: &str) -> bool {
        self.tracks
            .get(track)
            .is_some_and(|t| clip_number(&clip_base(t), name).is_some())
    }

    /// Names the MIDI clips that have no name, in the order a musician meets them: tracks
    /// in display order, then each track's clips by first placement (the unplaced last),
    /// then identifier; each takes its track's next default name. Returns how many it named.
    /// A project saved before clips were named takes this on opening: a normalisation of
    /// what is read, not an edit (no history).
    pub fn name_unnamed_clips(&mut self) -> usize {
        let mut todo: Vec<(TrackId, ClipId)> = Vec::new();
        for (tid, track) in self.tracks_in_order() {
            let mut unnamed: Vec<_> = track
                .clips
                .iter()
                .filter(|(_, c)| c.content.is_midi() && c.name.trim().is_empty())
                .map(|(id, _)| (first_start(track, id), id.clone()))
                .collect();
            unnamed.sort();
            todo.extend(unnamed.into_iter().map(|(_, id)| (tid.clone(), id)));
        }
        let named = todo.len();
        for (tid, cid) in todo {
            let name = self.next_clip_name(&tid);
            if let Some(clip) = self
                .tracks
                .get_mut(&tid)
                .and_then(|t| t.clips.get_mut(&cid))
            {
                clip.name = name;
            }
        }
        named
    }
}

/// The earliest start among a clip's placements, `None` (sorted last) when it has none.
fn first_start(track: &Track, clip: &ClipId) -> (bool, i64) {
    track
        .placements
        .values()
        .filter(|p| p.clip == *clip)
        .map(|p| p.start.get())
        .min()
        .map_or((true, 0), |s| (false, s))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copies_count_up_from_the_trailing_number() {
        assert_eq!(copy_name("Clip 2", []), "Clip 3");
        assert_eq!(copy_name("Clip 2", ["Clip 3", "Clip 4"]), "Clip 5");
        assert_eq!(copy_name("Synth", []), "Synth 2");
        assert_eq!(copy_name("TR 808", []), "TR 808 2");
        assert_eq!(copy_name("Kick 99", []), "Kick 100");
        assert_eq!(copy_name("5", []), "5 2");
    }

    #[test]
    fn a_long_name_loses_its_base_never_its_number() {
        let long = "x".repeat(MAX_NAME_CHARS);
        let copy = copy_name(&long, []);
        assert_eq!(copy.chars().count(), MAX_NAME_CHARS - 10 + 2);
        assert!(copy.ends_with(" 2"));
    }

    #[test]
    fn only_the_canonical_spelling_is_a_default_name() {
        assert_eq!(clip_number("Bass", "Bass 3"), Some(3));
        for other in [
            "Bass 03", "bass 3", "Bass 0", "Bass", "Bass 3 ", "Bass 3b", "My Bass 3",
        ] {
            assert_eq!(clip_number("Bass", other), None, "{other}");
        }
    }
}
