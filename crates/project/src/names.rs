//! Default names: the stable "Clip n" a clip gets when it is created, the rule a copy's
//! name follows, and the load-time pass that names the clips of a project saved before
//! clips were named.
//!
//! The rule of a copy is shared by every entity that is copied; the web app has the same
//! rule for tracks (`copyName` in `web/src/lib/project/tracks.ts`), kept in step by hand.

use crate::limits::MAX_NAME_CHARS;
use crate::model::{Project, Track};
use std::collections::BTreeSet;

/// The prefix of a clip's default name.
pub const CLIP_PREFIX: &str = "Clip";

/// The number of a default clip name ("Clip 3" is 3), `None` for any other name. Only the
/// canonical spelling counts: "Clip 03" and "clip 3" are the user's own names.
fn clip_number(name: &str) -> Option<u32> {
    let n: u32 = name
        .strip_prefix(CLIP_PREFIX)?
        .strip_prefix(' ')?
        .parse()
        .ok()?;
    (n > 0 && format!("{CLIP_PREFIX} {n}") == name).then_some(n)
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
    /// The name of the next new clip: "Clip n", n the first number no clip of the project
    /// has (a gap left by a deleted clip is filled).
    pub fn next_clip_name(&self) -> String {
        self.next_clip_name_beside(&[])
    }

    /// [`Project::next_clip_name`] when `also` are the names of clips a command is adding
    /// and the project does not hold yet.
    pub fn next_clip_name_beside(&self, also: &[String]) -> String {
        let used: BTreeSet<u32> = self
            .clip_names()
            .chain(also.iter().map(String::as_str))
            .filter_map(clip_number)
            .collect();
        let n = (1u32..).find(|n| !used.contains(n)).unwrap_or(u32::MAX);
        format!("{CLIP_PREFIX} {n}")
    }

    /// The name of the copy of a clip called `label`: [`copy_name`] against every clip name
    /// of the project and `also`. A clip with no name gets the next default name.
    pub fn copy_clip_name(&self, label: &str, also: &[String]) -> String {
        if label.trim().is_empty() {
            return self.next_clip_name_beside(also);
        }
        copy_name(
            label,
            self.clip_names().chain(also.iter().map(String::as_str)),
        )
    }

    fn clip_names(&self) -> impl Iterator<Item = &str> {
        self.tracks
            .values()
            .flat_map(|t| t.clips.values())
            .map(|c| c.name.as_str())
    }

    /// Names the MIDI clips that have no name, in the order a musician meets them: tracks
    /// in display order, then each track's clips by first placement (the unplaced last),
    /// then identifier. Returns how many it named. A project saved before clips were named
    /// takes this on opening: a normalisation of what is read, not an edit (no history).
    pub fn name_unnamed_clips(&mut self) -> usize {
        let mut todo: Vec<(crate::ids::TrackId, crate::ids::ClipId)> = Vec::new();
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
            let name = self.next_clip_name();
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
fn first_start(track: &Track, clip: &crate::ids::ClipId) -> (bool, i64) {
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
        assert_eq!(clip_number("Clip 3"), Some(3));
        for other in [
            "Clip 03",
            "clip 3",
            "Clip 0",
            "Clip",
            "Clip 3 ",
            "Clip 3b",
            "My Clip 3",
        ] {
            assert_eq!(clip_number(other), None, "{other}");
        }
    }
}
