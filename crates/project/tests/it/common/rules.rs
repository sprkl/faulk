//! The determinism rules of the writer (§ 2.4), checked on the bytes of a written folder.

use project::file::MemFs;

/// Panics when a file of the folder breaks a rule: UTF-8 without BOM, `\n` only and a final
/// `\n`, JSON keys sorted by byte order with one value per line, TSV lines sorted by their
/// natural key (the first column or two), no date and no volatile field.
pub fn check(fs: &MemFs) {
    for (path, bytes) in fs.files() {
        if path.starts_with("blobs/") || path.starts_with(".faulk/") {
            continue;
        }
        let text = std::str::from_utf8(bytes).unwrap_or_else(|_| panic!("{path}: not UTF-8"));
        assert!(!text.starts_with('\u{feff}'), "{path}: BOM");
        assert!(!text.contains('\r'), "{path}: carriage return");
        assert!(
            text.ends_with('\n') && !text.ends_with("\n\n"),
            "{path}: final newline"
        );
        if path.ends_with(".json") {
            sorted_keys(path, text);
        } else if path.ends_with(".tsv") {
            sorted_rows(path, text);
        }
    }
}

fn depth(line: &str) -> usize {
    (line.len() - line.trim_start().len()) / 2
}

fn sorted_keys(path: &str, text: &str) {
    let mut last: Vec<Option<String>> = Vec::new();
    for line in text.lines() {
        let d = depth(line);
        last.truncate(d + 1);
        while last.len() <= d {
            last.push(None);
        }
        let t = line.trim_start();
        // One value per line: a line is a key and a scalar or an opening, or a scalar of an
        // array, or a closing.
        if let Some(rest) = t.strip_prefix('"')
            && let Some(end) = key_end(rest)
            && rest[end..].starts_with("\": ")
        {
            let key = &rest[..end];
            if let Some(prev) = &last[d] {
                assert!(prev.as_str() < key, "{path}: key {key:?} after {prev:?}");
            }
            last[d] = Some(key.to_owned());
        }
        assert!(
            !t.contains("saved") && !t.contains("\"date\""),
            "{path}: volatile data"
        );
    }
}

/// The end of a JSON string's key, the byte index of the closing quote, honouring escapes.
fn key_end(s: &str) -> Option<usize> {
    let mut escaped = false;
    for (i, c) in s.char_indices() {
        match (escaped, c) {
            (true, _) => escaped = false,
            (false, '\\') => escaped = true,
            (false, '"') => return Some(i),
            _ => {}
        }
    }
    None
}

fn sorted_rows(path: &str, text: &str) {
    let mut rows = text.lines();
    let header = rows.next().unwrap();
    assert!(header.starts_with("# faulk-"), "{path}: header");
    let key = |line: &str| -> Vec<i64> {
        if path.ends_with(".notes.tsv") {
            // start, length, pitch, velocity: sorted by (start, pitch, length, velocity, ...).
            let f: Vec<i64> = line.split('\t').map(|v| v.parse().unwrap()).collect();
            vec![f[0], f[2], f[1]]
        } else if path.contains("/automation/") {
            vec![line.split('\t').next().unwrap().parse().unwrap()]
        } else {
            // Arrangement: start, then id.
            vec![line.split('\t').nth(1).unwrap().parse().unwrap()]
        }
    };
    let mut prev: Option<Vec<i64>> = None;
    for line in rows {
        let k = key(line);
        if let Some(p) = &prev {
            assert!(p <= &k, "{path}: rows out of order at {line:?}");
        }
        prev = Some(k);
    }
}
