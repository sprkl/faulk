//! Fractional order keys: the position of a track, a scene or a device among its
//! siblings (project model § 1.1).
//!
//! Each element carries its own sortable key instead of sitting in a central list, so two
//! branches that each insert an element touch different files and never conflict. A key
//! is an integer part then a fraction, compared bytewise:
//!
//! - the integer part starts with a head that gives its length: `a` to `z` are 2 to 27
//!   characters (non-negative integers, `a0` first), `A` to `Z` are 27 to 2 characters
//!   (negative integers, `Zz` just before `a0`); the other characters are base-62 digits;
//! - the fraction is base-62 digits (`0-9A-Za-z`, in byte order) that never ends with `0`,
//!   so there is always room for a key between two others.
//!
//! Appending or prepending moves the integer part (keys grow logarithmically); inserting
//! between two neighbours extends the fraction. Two branches can make the same key: the
//! order is then decided by the element's id ([`by_order`]), deterministically.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Base-62 digits in byte order.
const DIGITS: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
const ZERO: u8 = b'0';
const LAST: u8 = b'z';

/// Longest key accepted, in bytes: a safety limit, far above what insertions produce
/// (each insertion between two neighbours adds about one character to the fraction).
pub const MAX_ORDER_KEY_LEN: usize = 1024;

/// A fractional order key, validated on construction and on read.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct OrderKey(String);

/// Why a key cannot be read or made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OrderKeyError {
    /// Not a key: empty, a bad head or digit, a fraction ending with `0`, too long, or the
    /// smallest integer without a fraction (nothing can sort before it).
    Invalid(String),
    /// `between(a, b)` with `a >= b`.
    NotOrdered,
    /// No key exists past the largest integer or before the smallest one (`z…z` / `A0…0`
    /// with a fraction to grow): unreachable in practice.
    Exhausted,
}

impl fmt::Display for OrderKeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(key) => write!(f, "{key:?} is not an order key"),
            Self::NotOrdered => f.write_str("the lower key must sort before the upper key"),
            Self::Exhausted => f.write_str("no order key is left on that side"),
        }
    }
}

impl std::error::Error for OrderKeyError {}

fn digit(b: u8) -> Option<usize> {
    match b {
        b'0'..=b'9' => Some(usize::from(b - b'0')),
        b'A'..=b'Z' => Some(usize::from(b - b'A') + 10),
        b'a'..=b'z' => Some(usize::from(b - b'a') + 36),
        _ => None,
    }
}

/// Length of the integer part announced by its head.
fn integer_len(head: u8) -> Option<usize> {
    match head {
        b'a'..=b'z' => Some(usize::from(head - b'a') + 2),
        b'A'..=b'Z' => Some(usize::from(b'Z' - head) + 2),
        _ => None,
    }
}

/// The smallest integer part, `A` then 26 zeros: a key equal to it has no key before it.
const SMALLEST_INTEGER: &[u8] = b"A00000000000000000000000000";

/// Splits a valid key into (integer, fraction).
fn split(key: &[u8]) -> (&[u8], &[u8]) {
    let n = integer_len(key[0]).expect("a validated key has a head");
    key.split_at(n)
}

fn validate(key: &[u8]) -> bool {
    if key.is_empty() || key.len() > MAX_ORDER_KEY_LEN {
        return false;
    }
    let Some(n) = integer_len(key[0]) else {
        return false;
    };
    if key.len() < n || !key[1..].iter().all(|&b| digit(b).is_some()) {
        return false;
    }
    let (int, frac) = key.split_at(n);
    if frac.last() == Some(&ZERO) {
        return false;
    }
    !(frac.is_empty() && int == SMALLEST_INTEGER)
}

/// A key strictly between the fractions `a` and `b` (`b = None`: no upper bound), with
/// `a < b` as fractions, neither ending with `0`.
fn midpoint(a: &[u8], b: Option<&[u8]>) -> Vec<u8> {
    if let Some(b) = b {
        // Common prefix, `a` padded with zeros.
        let mut n = 0;
        while n < b.len() && a.get(n).copied().unwrap_or(ZERO) == b[n] {
            n += 1;
        }
        if n > 0 {
            let mut out = b[..n].to_vec();
            out.extend(midpoint(a.get(n..).unwrap_or(&[]), Some(&b[n..])));
            return out;
        }
    }
    let da = a.first().map_or(0, |&c| digit(c).expect("valid digit"));
    let db = b.map_or(DIGITS.len(), |b| digit(b[0]).expect("valid digit"));
    if db - da > 1 {
        // Round half up, as the reference algorithm does.
        vec![DIGITS[(da + db).div_ceil(2)]]
    } else if let Some(b) = b.filter(|b| b.len() > 1) {
        vec![b[0]]
    } else {
        let mut out = vec![DIGITS[da]];
        out.extend(midpoint(a.get(1..).unwrap_or(&[]), None));
        out
    }
}

fn increment_integer(int: &[u8]) -> Option<Vec<u8>> {
    let head = int[0];
    let mut digits = int[1..].to_vec();
    let mut carry = true;
    for d in digits.iter_mut().rev() {
        let next = digit(*d).expect("valid digit") + 1;
        if next == DIGITS.len() {
            *d = ZERO;
        } else {
            *d = DIGITS[next];
            carry = false;
            break;
        }
    }
    if !carry {
        let mut out = vec![head];
        out.extend(digits);
        return Some(out);
    }
    match head {
        b'Z' => Some(vec![b'a', ZERO]),
        b'z' => None,
        _ => {
            let h = head + 1;
            if h > b'a' {
                digits.push(ZERO);
            } else {
                digits.pop();
            }
            let mut out = vec![h];
            out.extend(digits);
            Some(out)
        }
    }
}

fn decrement_integer(int: &[u8]) -> Option<Vec<u8>> {
    let head = int[0];
    let mut digits = int[1..].to_vec();
    let mut borrow = true;
    for d in digits.iter_mut().rev() {
        let i = digit(*d).expect("valid digit");
        if i == 0 {
            *d = LAST;
        } else {
            *d = DIGITS[i - 1];
            borrow = false;
            break;
        }
    }
    if !borrow {
        let mut out = vec![head];
        out.extend(digits);
        return Some(out);
    }
    match head {
        b'a' => Some(vec![b'Z', LAST]),
        b'A' => None,
        _ => {
            let h = head - 1;
            if h < b'Z' {
                digits.push(LAST);
            } else {
                digits.pop();
            }
            let mut out = vec![h];
            out.extend(digits);
            Some(out)
        }
    }
}

fn concat(a: &[u8], b: &[u8]) -> Vec<u8> {
    let mut out = a.to_vec();
    out.extend_from_slice(b);
    out
}

impl OrderKey {
    /// Reads a key; refuses anything that is not a valid key.
    pub fn parse(s: &str) -> Result<Self, OrderKeyError> {
        if validate(s.as_bytes()) {
            Ok(Self(s.to_owned()))
        } else {
            Err(OrderKeyError::Invalid(s.to_owned()))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The key of the first element of an empty list: `a0`.
    pub fn first() -> Self {
        Self("a0".into())
    }

    /// A key strictly between `lower` and `upper`; `None` is an open end (`between(None,
    /// None)` is [`OrderKey::first`]). Errors when `lower >= upper`.
    pub fn between(
        lower: Option<&OrderKey>,
        upper: Option<&OrderKey>,
    ) -> Result<Self, OrderKeyError> {
        let key = match (lower, upper) {
            (None, None) => return Ok(Self::first()),
            (Some(a), Some(b)) if a >= b => return Err(OrderKeyError::NotOrdered),
            (None, Some(b)) => {
                let (ib, fb) = split(b.0.as_bytes());
                if ib == SMALLEST_INTEGER {
                    concat(ib, &midpoint(&[], Some(fb)))
                } else if !fb.is_empty() {
                    ib.to_vec()
                } else {
                    let i = decrement_integer(ib).ok_or(OrderKeyError::Exhausted)?;
                    if i == SMALLEST_INTEGER {
                        // The smallest integer alone is not a key: give it a fraction.
                        concat(&i, &midpoint(&[], None))
                    } else {
                        i
                    }
                }
            }
            (Some(a), None) => {
                let (ia, fa) = split(a.0.as_bytes());
                match increment_integer(ia) {
                    Some(i) => i,
                    None => concat(ia, &midpoint(fa, None)),
                }
            }
            (Some(a), Some(b)) => {
                let (ia, fa) = split(a.0.as_bytes());
                let (ib, fb) = split(b.0.as_bytes());
                if ia == ib {
                    concat(ia, &midpoint(fa, Some(fb)))
                } else {
                    let i = increment_integer(ia).ok_or(OrderKeyError::Exhausted)?;
                    if i.as_slice() < b.0.as_bytes() {
                        i
                    } else {
                        concat(ia, &midpoint(fa, None))
                    }
                }
            }
        };
        // Every key made here is checked like a key read from disk, so whatever `between`
        // returns reads back.
        if !validate(&key) {
            return Err(OrderKeyError::Exhausted);
        }
        Ok(Self(String::from_utf8(key).expect("keys are ASCII")))
    }

    /// The key that puts a new element at `index` (0 = first, `keys.len()` or more =
    /// last) among elements whose keys are `sorted`, in order. A command may receive an
    /// index; the model turns it into a key. Equal neighbours (two branches made the same
    /// key) are separated by the id tie-break, so the new key goes after both of them.
    pub fn for_index(sorted: &[&OrderKey], index: usize) -> Result<Self, OrderKeyError> {
        let index = index.min(sorted.len());
        let lower = index.checked_sub(1).map(|i| sorted[i]);
        let mut upper = sorted.get(index).copied();
        if let (Some(a), Some(b)) = (lower, upper)
            && a >= b
        {
            // Equal keys: there is no key strictly between; place after the lower one,
            // before the next strictly greater key.
            upper = sorted[index..].iter().copied().find(|k| *k > a);
        }
        Self::between(lower, upper)
    }
}

impl fmt::Display for OrderKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for OrderKey {
    type Error = OrderKeyError;
    fn try_from(s: String) -> Result<Self, OrderKeyError> {
        Self::parse(&s)
    }
}

impl From<OrderKey> for String {
    fn from(key: OrderKey) -> String {
        key.0
    }
}

/// The elements of a map sorted by (order key, id): the display order. Equal keys (made
/// independently on two branches) are tie-broken by id, so the order is deterministic
/// and a merge never conflicts on it (project model § 4.2).
pub fn by_order<'a, K: Ord + 'a, V: 'a>(
    items: impl IntoIterator<Item = (&'a K, &'a V)>,
    order: impl Fn(&V) -> &OrderKey,
) -> Vec<(&'a K, &'a V)> {
    let mut v: Vec<_> = items.into_iter().collect();
    v.sort_by(|(ka, a), (kb, b)| order(a).cmp(order(b)).then_with(|| ka.cmp(kb)));
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn k(s: &str) -> OrderKey {
        OrderKey::parse(s).unwrap()
    }

    #[test]
    fn reference_sequence() {
        // The sequence of the reference algorithm (fractional indexing, base 62).
        assert_eq!(OrderKey::between(None, None).unwrap(), k("a0"));
        assert_eq!(OrderKey::between(Some(&k("a0")), None).unwrap(), k("a1"));
        assert_eq!(OrderKey::between(None, Some(&k("a0"))).unwrap(), k("Zz"));
        assert_eq!(
            OrderKey::between(Some(&k("a0")), Some(&k("a1"))).unwrap(),
            k("a0V")
        );
        assert_eq!(
            OrderKey::between(Some(&k("a0V")), Some(&k("a1"))).unwrap(),
            k("a0l")
        );
        assert_eq!(OrderKey::between(Some(&k("az")), None).unwrap(), k("b00"));
        assert_eq!(OrderKey::between(None, Some(&k("b00"))).unwrap(), k("az"));
        assert_eq!(OrderKey::between(Some(&k("Zz")), None).unwrap(), k("a0"));
        assert_eq!(
            OrderKey::between(Some(&k("a0")), Some(&k("a0V"))).unwrap(),
            k("a0G")
        );
        assert_eq!(OrderKey::between(None, Some(&k("a0V"))).unwrap(), k("a0"));
    }

    #[test]
    fn refuses_invalid_keys() {
        for bad in [
            "",
            "a",
            "b0",
            "a00",
            "a0V0",
            "0a",
            "a-",
            "é0",
            "a0 ",
            "A00000000000000000000000000",
        ] {
            assert!(OrderKey::parse(bad).is_err(), "{bad:?} accepted");
        }
        assert!(OrderKey::parse(&format!("a0{}", "1".repeat(MAX_ORDER_KEY_LEN))).is_err());
        assert!(OrderKey::parse("A000000000000000000000000001").is_ok());
        assert!(serde_json::from_str::<OrderKey>("\"a00\"").is_err());
        assert_eq!(
            serde_json::to_string(&k("a0V")).unwrap(),
            "\"a0V\"",
            "a key is a plain string on disk"
        );
    }

    #[test]
    fn between_refuses_unordered_bounds() {
        assert_eq!(
            OrderKey::between(Some(&k("a1")), Some(&k("a0"))),
            Err(OrderKeyError::NotOrdered)
        );
        assert_eq!(
            OrderKey::between(Some(&k("a1")), Some(&k("a1"))),
            Err(OrderKeyError::NotOrdered)
        );
    }

    #[test]
    fn the_ends_of_the_integer_range() {
        let largest = k(&format!("z{}", "z".repeat(26)));
        // No larger integer: the fraction grows instead.
        let after = OrderKey::between(Some(&largest), None).unwrap();
        assert!(after > largest);
        let smallest = k("A000000000000000000000000001");
        let before = OrderKey::between(None, Some(&smallest)).unwrap();
        assert!(before < smallest);
        assert!(OrderKey::parse(before.as_str()).is_ok());
        // One integer above the smallest: decrementing reaches the smallest integer,
        // which alone is not a key; the result carries a fraction and reads back.
        let next = k("A00000000000000000000000001");
        let before = OrderKey::between(None, Some(&next)).unwrap();
        assert!(before < next);
        assert_eq!(OrderKey::parse(before.as_str()), Ok(before.clone()));
        let first = OrderKey::between(None, Some(&before)).unwrap();
        assert!(first < before && OrderKey::parse(first.as_str()).is_ok());
    }

    #[test]
    fn appends_and_prepends_stay_short() {
        let mut last = OrderKey::first();
        for _ in 0..10_000 {
            last = OrderKey::between(Some(&last), None).unwrap();
        }
        assert!(last.as_str().len() <= 4, "{last}");
        let mut first = OrderKey::first();
        for _ in 0..10_000 {
            first = OrderKey::between(None, Some(&first)).unwrap();
        }
        assert!(first.as_str().len() <= 4, "{first}");
    }

    #[test]
    fn for_index_places_at_the_index() {
        let a = k("a0");
        let b = k("a1");
        let c = k("a2");
        let keys = [&a, &b, &c];
        assert!(OrderKey::for_index(&keys, 0).unwrap() < a);
        let at1 = OrderKey::for_index(&keys, 1).unwrap();
        assert!(a < at1 && at1 < b);
        assert!(OrderKey::for_index(&keys, 3).unwrap() > c);
        assert!(OrderKey::for_index(&keys, 99).unwrap() > c);
        assert_eq!(OrderKey::for_index(&[], 5).unwrap(), OrderKey::first());
        // Two equal keys (from two branches): the new one goes after both, before c.
        let dup = [&a, &b, &b, &c];
        let mid = OrderKey::for_index(&dup, 2).unwrap();
        assert!(b < mid && mid < c, "{mid}");
    }

    #[test]
    fn by_order_breaks_ties_by_id() {
        let items = [(2, k("a0")), (1, k("a0")), (0, k("a1"))];
        let sorted: Vec<_> = by_order(items.iter().map(|(i, k)| (i, k)), |k| k)
            .into_iter()
            .map(|(i, _)| *i)
            .collect();
        assert_eq!(sorted, [1, 2, 0]);
    }

    proptest! {
        /// Any sequence of insertions keeps a valid, strictly sorted list, each new key
        /// strictly between its neighbours.
        #[test]
        fn random_insertions_stay_sorted(positions in prop::collection::vec(any::<prop::sample::Index>(), 1..300)) {
            let mut keys: Vec<OrderKey> = Vec::new();
            for p in positions {
                let i = p.index(keys.len() + 1);
                let refs: Vec<&OrderKey> = keys.iter().collect();
                let key = OrderKey::for_index(&refs, i).unwrap();
                prop_assert!(OrderKey::parse(key.as_str()).is_ok());
                if i > 0 { prop_assert!(keys[i - 1] < key); }
                if i < keys.len() { prop_assert!(key < keys[i]); }
                keys.insert(i, key);
            }
            prop_assert!(keys.windows(2).all(|w| w[0] < w[1]));
        }

        /// With duplicate keys (two branches appended at the same place, then a line
        /// merge), inserting at any index still succeeds: the new key sorts after the
        /// lower neighbour and before the next strictly greater key.
        #[test]
        fn insertions_next_to_duplicates_never_fail(
            ops in prop::collection::vec((any::<prop::sample::Index>(), any::<bool>()), 1..200)
        ) {
            let mut keys: Vec<OrderKey> = vec![OrderKey::first()];
            for (p, duplicate) in ops {
                if duplicate {
                    let i = p.index(keys.len());
                    let copy = keys[i].clone();
                    keys.insert(i, copy);
                    continue;
                }
                let i = p.index(keys.len() + 1);
                let refs: Vec<&OrderKey> = keys.iter().collect();
                let key = OrderKey::for_index(&refs, i).unwrap();
                if i > 0 { prop_assert!(keys[i - 1] < key); }
                let pos = keys.partition_point(|k| *k < key);
                prop_assert!(keys.get(pos).is_none_or(|k| *k != key));
                keys.insert(pos, key);
                prop_assert!(keys.windows(2).all(|w| w[0] <= w[1]));
            }
        }

        /// `between` of two valid ordered keys is strictly between them.
        #[test]
        fn between_is_strictly_between(steps in prop::collection::vec(0u8..3, 1..60)) {
            // Build two ordered keys from a random walk, then check the midpoint.
            let mut lo = OrderKey::first();
            let mut hi = OrderKey::between(Some(&lo), None).unwrap();
            for s in steps {
                let mid = OrderKey::between(Some(&lo), Some(&hi)).unwrap();
                prop_assert!(lo < mid && mid < hi);
                match s {
                    0 => lo = mid,
                    1 => hi = mid,
                    _ => {
                        hi = OrderKey::between(Some(&hi), None).unwrap();
                    }
                }
            }
        }

        /// Parsing accepts exactly what `parse` validates, and keeps it unchanged.
        #[test]
        fn parse_round_trips(s in "[a-zA-Z][0-9A-Za-z]{0,30}") {
            if let Ok(key) = OrderKey::parse(&s) {
                prop_assert_eq!(key.as_str(), s.as_str());
                let json = serde_json::to_string(&key).unwrap();
                prop_assert_eq!(serde_json::from_str::<OrderKey>(&json).unwrap(), key);
            }
        }
    }
}
