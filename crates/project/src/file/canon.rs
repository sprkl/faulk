//! The canonical JSON writer (§ 2.4): UTF-8, keys sorted by byte order, two-space
//! indentation, one value per line, a final `\n`. The same value always gives the same
//! bytes, whatever the features `serde_json` was built with.

use serde::Serialize;
use serde_json::Value;
use std::fmt::Write as _;

/// A value as canonical JSON, or why it does not serialize (a non-finite number).
pub fn json<T: Serialize>(value: &T) -> Result<Vec<u8>, String> {
    let value = serde_json::to_value(value).map_err(|e| e.to_string())?;
    let mut out = String::new();
    print(&value, 0, &mut out)?;
    out.push('\n');
    Ok(out.into_bytes())
}

fn number(n: &serde_json::Number) -> Result<String, String> {
    if n.is_f64() {
        let f = n.as_f64().ok_or("number")?;
        if !f.is_finite() {
            return Err("a non-finite number".into());
        }
        // Never `-0` (§ 2.5).
        if f == 0.0 {
            return Ok("0.0".into());
        }
    }
    Ok(n.to_string())
}

fn string(s: &str, out: &mut String) -> Result<(), String> {
    out.push_str(&serde_json::to_string(s).map_err(|e| e.to_string())?);
    Ok(())
}

fn indent(depth: usize, out: &mut String) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

fn print(value: &Value, depth: usize, out: &mut String) -> Result<(), String> {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => out.push_str(&number(n)?),
        Value::String(s) => string(s, out)?,
        Value::Array(items) if items.is_empty() => out.push_str("[]"),
        Value::Array(items) => {
            out.push_str("[\n");
            for (i, item) in items.iter().enumerate() {
                indent(depth + 1, out);
                print(item, depth + 1, out)?;
                out.push_str(if i + 1 < items.len() { ",\n" } else { "\n" });
            }
            indent(depth, out);
            out.push(']');
        }
        Value::Object(map) if map.is_empty() => out.push_str("{}"),
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push_str("{\n");
            for (i, key) in keys.iter().enumerate() {
                indent(depth + 1, out);
                string(key, out)?;
                out.push_str(": ");
                if let Some(v) = map.get(*key) {
                    print(v, depth + 1, out)?;
                }
                out.push_str(if i + 1 < keys.len() { ",\n" } else { "\n" });
            }
            indent(depth, out);
            out.push('}');
        }
    }
    Ok(())
}

/// A fast 64-bit hash of some bytes, for telling whether a file's bytes changed within one
/// process or one cache snapshot. Not cryptographic and not stable across versions of the
/// format: blobs are named by SHA-256, never by this.
pub fn fast_hash(bytes: &[u8]) -> u64 {
    const K: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut h = (bytes.len() as u64).wrapping_mul(K) ^ 0x2545_F491_4F6C_DD1D;
    let (chunks, rest) = bytes.as_chunks::<8>();
    for c in chunks {
        h = (h.rotate_left(23) ^ u64::from_le_bytes(*c)).wrapping_mul(K);
    }
    let mut tail = [0u8; 8];
    tail[..rest.len()].copy_from_slice(rest);
    h = (h.rotate_left(23) ^ u64::from_le_bytes(tail)).wrapping_mul(K);
    h ^ (h >> 32)
}

/// Writes a number of the project the way the text files do: the shortest decimal that
/// reads back as the same `f64`, never `-0`.
pub fn float(v: f64, out: &mut String) {
    let v = if v == 0.0 { 0.0 } else { v };
    let _ = write!(out, "{v}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json as j;

    #[test]
    fn sorts_keys_and_indents() {
        let v = j!({"b": [1, 2], "a": {"z": null, "y": true}, "c": [], "d": {}});
        let text = String::from_utf8(json(&v).unwrap()).unwrap();
        assert_eq!(
            text,
            "{\n  \"a\": {\n    \"y\": true,\n    \"z\": null\n  },\n  \"b\": [\n    1,\n    2\n  ],\n  \"c\": [],\n  \"d\": {}\n}\n"
        );
    }

    #[test]
    fn negative_zero_is_zero() {
        assert_eq!(json(&-0.0f64).unwrap(), b"0.0\n");
        assert_eq!(json(&0.5f64).unwrap(), b"0.5\n");
        let mut s = String::new();
        float(-0.0, &mut s);
        assert_eq!(s, "0");
    }

    #[test]
    fn refuses_non_finite() {
        // `serde_json` turns a non-finite number into `null`: the writer sees the null,
        // so the types of the document refuse a non-finite number themselves
        // (`serde_util::finite`); this only checks that nothing panics.
        assert!(json(&f64::NAN).is_ok());
    }

    #[test]
    fn hash_tells_bytes_apart() {
        assert_ne!(fast_hash(b"abc"), fast_hash(b"abd"));
        assert_ne!(fast_hash(b""), fast_hash(b"\0"));
        assert_ne!(fast_hash(b"12345678"), fast_hash(b"123456789"));
        assert_eq!(fast_hash(b"same"), fast_hash(b"same"));
    }
}
