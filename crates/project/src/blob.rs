//! Content-addressed blobs (project model § 3): audio, very dense automation, opaque
//! instrument state. A blob is named by the SHA-256 of its bytes and never modified; the
//! project references it by that hash, never by a path. Computing the hash is the
//! storage's job (SV-1, AU-1): this crate only carries it.

use serde::{Deserialize, Serialize};
use std::fmt;

/// The SHA-256 of a blob, written as 64 lower-case hexadecimal characters (the same
/// `oid` Git LFS uses).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, std::hash::Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Hash(pub [u8; 32]);

/// Why a string is not a [`Hash`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HashError(pub String);

impl fmt::Display for HashError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:?} is not a SHA-256 (64 lower-case hexadecimal characters)",
            self.0
        )
    }
}

impl std::error::Error for HashError {}

impl Hash {
    /// Reads 64 lower-case hexadecimal characters.
    pub fn parse(s: &str) -> Result<Self, HashError> {
        let bytes = s.as_bytes();
        if bytes.len() != 64 {
            return Err(HashError(s.to_owned()));
        }
        let nibble = |c: u8| match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            _ => None,
        };
        let mut out = [0u8; 32];
        for (i, [h, l]) in bytes.as_chunks::<2>().0.iter().enumerate() {
            match (nibble(*h), nibble(*l)) {
                (Some(h), Some(l)) => out[i] = (h << 4) | l,
                _ => return Err(HashError(s.to_owned())),
            }
        }
        Ok(Self(out))
    }
}

impl fmt::Display for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for b in self.0 {
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Hash({self})")
    }
}

impl TryFrom<String> for Hash {
    type Error = HashError;
    fn try_from(s: String) -> Result<Self, HashError> {
        Self::parse(&s)
    }
}

impl From<Hash> for String {
    fn from(h: Hash) -> String {
        h.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn reads_lower_case_hex_only() {
        // SHA-256 of the empty string.
        let empty = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        let h = Hash::parse(empty).unwrap();
        assert_eq!(h.0[0], 0xe3);
        assert_eq!(h.to_string(), empty);
        assert!(Hash::parse(&empty.to_uppercase()).is_err());
        assert!(Hash::parse(&empty[..63]).is_err());
        assert!(Hash::parse(&format!("{empty}0")).is_err());
        assert!(Hash::parse(&format!("{}g", &empty[..63])).is_err());
        assert!(serde_json::from_str::<Hash>("\"abc\"").is_err());
    }

    proptest! {
        #[test]
        fn display_parse_round_trip(bytes in any::<[u8; 32]>()) {
            let h = Hash(bytes);
            prop_assert_eq!(Hash::parse(&h.to_string()).unwrap(), h);
            let json = serde_json::to_string(&h).unwrap();
            prop_assert_eq!(serde_json::from_str::<Hash>(&json).unwrap(), h);
        }
    }
}
