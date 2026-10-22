//! Timing helpers: run a closure, keep its duration, summarise a series.

use std::time::{Duration, Instant};

pub fn time<T>(f: impl FnOnce() -> T) -> (T, Duration) {
    let t = Instant::now();
    let out = f();
    (out, t.elapsed())
}

pub fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

pub fn us(d: Duration) -> f64 {
    d.as_secs_f64() * 1e6
}

/// Runs `f` `n` times and returns the durations in milliseconds, in run order.
pub fn series(n: usize, mut f: impl FnMut()) -> Vec<f64> {
    (0..n).map(|_| ms(time(&mut f).1)).collect()
}

pub fn median(v: &[f64]) -> f64 {
    percentile(v, 0.5)
}

pub fn percentile(v: &[f64], p: f64) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.total_cmp(b));
    let at = ((s.len() - 1) as f64 * p).round() as usize;
    s[at]
}

pub fn max(v: &[f64]) -> f64 {
    v.iter().copied().fold(f64::NAN, f64::max)
}

/// "median 12.3 ms (min 11.9, max 14.0, n=5)".
pub fn describe(v: &[f64], unit: &str) -> String {
    let min = v.iter().copied().fold(f64::INFINITY, f64::min);
    format!(
        "median {:.2} {unit} (min {:.2}, max {:.2}, n={})",
        median(v),
        min,
        max(v),
        v.len()
    )
}

pub fn mib(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

pub fn gib(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0 * 1024.0)
}
