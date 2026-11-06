//! Format versions and migrations (§ 2.6). Faulk reads every version it has a migration
//! for and writes only the current one; a newer version is refused.
//!
//! A migration is a pure function on the text files of the folder held in memory (the
//! blobs are never touched: they are named by their content). The host saves a migrated
//! project as a version of its own, so that the diff of the migration is not mixed with
//! musical changes.

use super::error::FileError;
use super::fs::MemFs;

/// One step: the folder in format `from` becomes the folder in format `from + 1`. The
/// function must also set `format` in `project.json`.
#[derive(Clone, Copy)]
pub struct Migration {
    pub from: u32,
    pub apply: fn(&mut MemFs) -> Result<(), String>,
}

/// The migrations of this build, one per format below [`crate::FORMAT_VERSION`].
pub const MIGRATIONS: &[Migration] = &[
    Migration {
        from: 1,
        apply: v1_to_v2,
    },
    Migration {
        from: 2,
        apply: v2_to_v3,
    },
    Migration {
        from: 3,
        apply: v3_to_v4,
    },
    Migration {
        from: 4,
        apply: v4_to_v5,
    },
    Migration {
        from: 5,
        apply: v5_to_v6,
    },
];

/// Format 6 (#219): a track's chain may hold MIDI effects (role `midi_effect`). Nothing in the
/// files changes (a project of format 5 has none); the number does, so that a build of format 5
/// reports the folder as made by a newer build.
fn v5_to_v6(mem: &mut MemFs) -> Result<(), String> {
    set_format(mem, 6)
}

/// Format 5 (#695): the project may hold a key and scale (`scale`). Nothing in the files
/// changes (a project of format 4 has no scale); the number does, so that a build of format 4
/// reports the folder as made by a newer build.
fn v4_to_v5(mem: &mut MemFs) -> Result<(), String> {
    set_format(mem, 5)
}

/// Format 4 (#191): a device may name its preset and the folder may hold `presets/`. Nothing
/// in the files changes; the number does, so that a build of format 3 reports the folder as
/// made by a newer build.
fn v3_to_v4(mem: &mut MemFs) -> Result<(), String> {
    set_format(mem, 4)
}

/// Sets `format` in `project.json` and changes nothing else.
fn set_format(mem: &mut MemFs, format: u32) -> Result<(), String> {
    use serde_json::Value;
    let mut project: Value = serde_json::from_slice(
        mem.files()
            .get("project.json")
            .ok_or("project.json is missing")?,
    )
    .map_err(|e| format!("project.json: {e}"))?;
    project["format"] = Value::from(format);
    let bytes = serde_json::to_vec_pretty(&project).map_err(|e| e.to_string())?;
    mem.insert("project.json", bytes);
    Ok(())
}

/// Format 3 (#490): tracks may hold a `height`. Nothing in the files changes; the number
/// does, so that a build of format 2 reports the folder as made by a newer build.
fn v2_to_v3(mem: &mut MemFs) -> Result<(), String> {
    use serde_json::Value;
    let mut project: Value = serde_json::from_slice(
        mem.files()
            .get("project.json")
            .ok_or("project.json is missing")?,
    )
    .map_err(|e| format!("project.json: {e}"))?;
    project["format"] = Value::from(3);
    let bytes = serde_json::to_vec_pretty(&project).map_err(|e| e.to_string())?;
    mem.insert("project.json", bytes);
    Ok(())
}

/// Format 2 (#473): in the arrangement, a placement longer than its MIDI clip repeats the
/// clip, even one without a loop; in format 1 such a clip was silent after its end. So that
/// a song written before sounds the same, a placement of a MIDI clip without a loop is cut
/// at the clip's end (what lay past it was silence). A clip with a loop repeated it already
/// and is left alone.
fn v1_to_v2(mem: &mut MemFs) -> Result<(), String> {
    use serde_json::Value;
    let mut project: Value = serde_json::from_slice(
        mem.files()
            .get("project.json")
            .ok_or("project.json is missing")?,
    )
    .map_err(|e| format!("project.json: {e}"))?;
    project["format"] = Value::from(2);
    let bytes = serde_json::to_vec_pretty(&project).map_err(|e| e.to_string())?;
    mem.insert("project.json", bytes);

    let arrangements: Vec<String> = mem
        .files()
        .keys()
        .filter(|p| p.starts_with("tracks/") && p.ends_with("/arrangement.tsv"))
        .cloned()
        .collect();
    for path in arrangements {
        let track_dir = path.trim_end_matches("arrangement.tsv").to_owned();
        let text = String::from_utf8(mem.files()[&path].clone())
            .map_err(|_| format!("{path}: not UTF-8"))?;
        let mut out = String::with_capacity(text.len());
        let mut changed = false;
        for line in text.split_inclusive('\n') {
            let row = line.trim_end_matches(['\n', '\r']);
            let cols: Vec<&str> = row.split('\t').collect();
            let cut = if row.starts_with('#') || cols.len() != 5 {
                None
            } else {
                one_shot_length(mem, &track_dir, cols[3]).and_then(|clip_len| {
                    let length: i64 = cols[2].parse().ok()?;
                    let offset: i64 = cols[4].parse().ok()?;
                    (offset.checked_add(length)? > clip_len).then_some((clip_len, offset))
                })
            };
            match cut {
                // An offset at or past the clip's end is invalid in any format: left as it
                // is, for the reader to refuse the folder as corrupt.
                Some((clip_len, offset)) if offset < clip_len => {
                    changed = true;
                    let ending = &line[row.len()..];
                    out.push_str(&format!(
                        "{}\t{}\t{}\t{}\t{}{ending}",
                        cols[0],
                        cols[1],
                        clip_len - offset,
                        cols[3],
                        cols[4]
                    ));
                }
                _ => out.push_str(line),
            }
        }
        if changed {
            mem.insert(path, out.into_bytes());
        }
    }
    Ok(())
}

/// The length of a MIDI clip of a track that has no loop, from its file; `None` for an
/// audio clip, a clip with a loop, or a file that does not read (the reader refuses it
/// with its own error afterwards).
fn one_shot_length(mem: &MemFs, track_dir: &str, clip: &str) -> Option<i64> {
    let bytes = mem.files().get(&format!("{track_dir}clips/{clip}.json"))?;
    let value: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    let midi = value.get("content")?.get("midi")?;
    if !midi.get("looping").is_none_or(serde_json::Value::is_null) {
        return None;
    }
    midi.get("length")?.as_i64()
}

/// Applies the steps from `found` up to `target`. A step that is not in `table` is an
/// unsupported format; a step that fails is a typed error, and `mem` must then be dropped.
pub fn migrate(
    table: &[Migration],
    found: u32,
    target: u32,
    mem: &mut MemFs,
) -> Result<(), FileError> {
    let mut version = found;
    while version < target {
        let step = table
            .iter()
            .find(|m| m.from == version)
            .ok_or(FileError::UnsupportedFormat { found })?;
        (step.apply)(mem).map_err(|message| FileError::Migration {
            from: version,
            message,
        })?;
        version += 1;
    }
    Ok(())
}
