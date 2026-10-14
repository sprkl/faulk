//! Serde helpers of the document: maps that refuse duplicate keys, and decibels that
//! write `-inf` as `null` (project model § 2.5).

use serde::de::{MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::BTreeMap;
use std::fmt;
use std::marker::PhantomData;

/// Reads a map and refuses a key that appears twice. `serde_json` would otherwise keep
/// the last value silently, so two entities with the same id, or two automation points
/// on the same tick, would vanish into one instead of being refused.
pub fn unique_map<'de, D, K, V>(deserializer: D) -> Result<BTreeMap<K, V>, D::Error>
where
    D: Deserializer<'de>,
    K: Deserialize<'de> + Ord + fmt::Debug,
    V: Deserialize<'de>,
{
    struct UniqueMap<K, V>(PhantomData<(K, V)>);

    impl<'de, K, V> Visitor<'de> for UniqueMap<K, V>
    where
        K: Deserialize<'de> + Ord + fmt::Debug,
        V: Deserialize<'de>,
    {
        type Value = BTreeMap<K, V>;

        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("a map without duplicate keys")
        }

        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
            let mut out = BTreeMap::new();
            while let Some((key, value)) = map.next_entry::<K, V>()? {
                if out.contains_key(&key) {
                    return Err(serde::de::Error::custom(format_args!(
                        "duplicate key {key:?}"
                    )));
                }
                out.insert(key, value);
            }
            Ok(out)
        }
    }

    deserializer.deserialize_map(UniqueMap(PhantomData))
}

/// Writes a finite number, `-0` as `0` (project model § 2.5); refuses NaN and infinities
/// instead of letting `serde_json` write them as `null`, which would hide a corrupt value.
fn write_finite<S: Serializer>(v: f64, s: S) -> Result<S::Ok, S::Error> {
    if !v.is_finite() {
        return Err(serde::ser::Error::custom(format_args!(
            "{v} is not a finite number"
        )));
    }
    s.serialize_f64(if v == 0.0 { 0.0 } else { v })
}

/// A finite number: NaN and infinities are refused on write, `-0` is written `0`.
pub mod finite {
    use super::*;

    pub fn serialize<S: Serializer>(v: &f64, s: S) -> Result<S::Ok, S::Error> {
        write_finite(*v, s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
        f64::deserialize(d)
    }
}

/// A map of finite numbers (device parameters): see [`finite`]; duplicate keys refused.
pub mod finite_map {
    use super::*;
    use serde::ser::SerializeMap;

    pub fn serialize<S: Serializer>(m: &BTreeMap<String, f64>, s: S) -> Result<S::Ok, S::Error> {
        struct F(f64);
        impl Serialize for F {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                write_finite(self.0, s)
            }
        }
        let mut map = s.serialize_map(Some(m.len()))?;
        for (k, v) in m {
            map.serialize_entry(k, &F(*v))?;
        }
        map.end()
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<BTreeMap<String, f64>, D::Error> {
        unique_map(d)
    }
}

/// A gain in decibels: `-inf` (silence) is written `null`, any other value as a finite
/// number (`-0` as `0`); NaN and `+inf` are refused on write.
pub mod db {
    use super::*;

    pub fn serialize<S: Serializer>(v: &f64, s: S) -> Result<S::Ok, S::Error> {
        if *v == f64::NEG_INFINITY {
            s.serialize_none()
        } else {
            write_finite(*v, s)
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
        Ok(Option::<f64>::deserialize(d)?.unwrap_or(f64::NEG_INFINITY))
    }
}

/// A colour, written `#rrggbb`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// Reads `#rrggbb` (lower or upper case).
    pub fn parse(s: &str) -> Option<Self> {
        let hex = s.strip_prefix('#')?;
        if hex.len() != 6 || !hex.bytes().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
        Some(Self::new(byte(0)?, byte(2)?, byte(4)?))
    }
}

impl fmt::Display for Rgb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }
}

impl Serialize for Rgb {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Rgb {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Rgb::parse(&s).ok_or_else(|| serde::de::Error::custom(format_args!("{s:?} is not #rrggbb")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Deserialize)]
    struct M {
        #[serde(deserialize_with = "unique_map")]
        m: BTreeMap<String, i32>,
    }

    #[test]
    fn duplicate_keys_are_refused() {
        let ok = serde_json::from_str::<M>(r#"{"m":{"a":1,"b":2}}"#).unwrap();
        assert_eq!(ok.m.len(), 2);
        let err = serde_json::from_str::<M>(r#"{"m":{"a":1,"a":2}}"#).unwrap_err();
        assert!(err.to_string().contains("duplicate key"), "{err}");
    }

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct G {
        #[serde(with = "db")]
        g: f64,
    }

    #[test]
    fn minus_infinity_db_is_null() {
        assert_eq!(
            serde_json::to_string(&G {
                g: f64::NEG_INFINITY
            })
            .unwrap(),
            r#"{"g":null}"#
        );
        assert_eq!(
            serde_json::from_str::<G>(r#"{"g":null}"#).unwrap().g,
            f64::NEG_INFINITY
        );
        assert_eq!(
            serde_json::to_string(&G { g: -6.5 }).unwrap(),
            r#"{"g":-6.5}"#
        );
        assert_eq!(
            serde_json::to_string(&G { g: -0.0 }).unwrap(),
            r#"{"g":0.0}"#
        );
        // NaN and +inf would be written `null` and read back as -inf: refused instead.
        assert!(serde_json::to_string(&G { g: f64::NAN }).is_err());
        assert!(serde_json::to_string(&G { g: f64::INFINITY }).is_err());
    }

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct F {
        #[serde(with = "finite")]
        v: f64,
        #[serde(with = "finite_map")]
        m: BTreeMap<String, f64>,
    }

    #[test]
    fn finite_numbers_only() {
        let f = |v: f64, p: f64| F {
            v,
            m: BTreeMap::from([("p".to_owned(), p)]),
        };
        assert_eq!(
            serde_json::to_string(&f(-0.0, -0.0)).unwrap(),
            r#"{"v":0.0,"m":{"p":0.0}}"#
        );
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(serde_json::to_string(&f(bad, 1.0)).is_err());
            assert!(serde_json::to_string(&f(1.0, bad)).is_err());
        }
        assert!(serde_json::from_str::<F>(r#"{"v":null,"m":{}}"#).is_err());
        assert!(serde_json::from_str::<F>(r#"{"v":1,"m":{"p":1,"p":2}}"#).is_err());
    }

    #[test]
    fn colours() {
        let c = Rgb::parse("#FF8000").unwrap();
        assert_eq!(c, Rgb::new(255, 128, 0));
        assert_eq!(serde_json::to_string(&c).unwrap(), "\"#ff8000\"");
        for bad in ["ff8000", "#ff800", "#ff80000", "#gg8000", "#+f8000"] {
            assert!(Rgb::parse(bad).is_none(), "{bad}");
        }
    }
}
