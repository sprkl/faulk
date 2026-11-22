//! Issue #686: the Filter at its new defaults (no LFO depth, no envelope amount, no drive)
//! renders the very same samples as the Filter of DV-2 did before the movement existed,
//! frozen in `reference/old_filter.rs`: bit for bit, on a scripted render that moves every
//! parameter the old filter had (every mode, ramps across the range, resonance, mix), at
//! several sample rates and block sizes, playing or not.

#[path = "reference/old_filter.rs"]
mod old_filter;

use engine::device::{AudioIo, BlockCtx, Device, Event, EventBuffer, Transport};
use engine::devices::filter::{CUTOFF, DESCRIPTOR, Filter, MIX, MODE, RESONANCE};
use engine::msg::ParamId;

fn noise(len: usize, seed: u32) -> Vec<f32> {
    let mut s = seed;
    (0..len)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            (s as f32 / u32::MAX as f32) * 2.0 - 1.0
        })
        .collect()
}

fn ev(id: u32, value: f32) -> Event {
    Event::ParamValue {
        offset: 0,
        id: ParamId(id),
        value,
    }
}

/// The script: every mode, ramps of the cutoff over the whole range, resonance from 0 to 1,
/// mix changes, mode changes mid-ramp, a burst of cutoff changes every 53 frames.
fn script() -> Vec<(usize, Event)> {
    let mut script: Vec<(usize, Event)> = vec![(0, ev(RESONANCE, 0.4))];
    let cutoffs = [20.0, 20_000.0, 440.0, 441.0, 3000.0, 80.0, 12_000.0, 1000.0];
    for (i, c) in cutoffs.iter().enumerate() {
        script.push((3_000 + i * 31_013 % 250_000, ev(CUTOFF, *c)));
    }
    for (i, m) in [1.0, 2.0, 0.0, 2.0, 1.0, 0.0].iter().enumerate() {
        script.push((10_000 + i * 41_000, ev(MODE, *m)));
    }
    for (i, r) in [0.0, 1.0, 0.7, 0.05, 1.0].iter().enumerate() {
        script.push((7_000 + i * 50_000, ev(RESONANCE, *r)));
    }
    for (i, m) in [0.5, 0.0, 1.0, 0.3].iter().enumerate() {
        script.push((20_000 + i * 60_000, ev(MIX, *m)));
    }
    for k in 0..40 {
        script.push((
            150_000 + k * 53,
            ev(CUTOFF, 100.0 + ((k * 7919) % 9000) as f32),
        ));
    }
    script.sort_by_key(|(at, _)| *at);
    script
}

/// 6 s of stereo noise through `f` in blocks of `block` frames, with `extra` events at frame 0.
fn render(f: &mut dyn Device, rate: f32, block: usize, play: bool, extra: &[Event]) -> Vec<f32> {
    let total = (rate * 6.0) as usize;
    let (src_l, src_r) = (noise(total, 5), noise(total, 77));
    let mut script = script();
    for e in extra {
        script.insert(0, (0, *e));
    }
    let mut out = Vec::with_capacity(2 * total);
    let mut unused = EventBuffer::try_with_capacity(1).unwrap();
    let mut next = 0;
    let mut at = 0;
    while at < total {
        let n = block.min(total - at);
        let mut events = Vec::new();
        while next < script.len() && script[next].0 < at + n {
            let mut e = script[next].1;
            if let Event::ParamValue { offset, .. } = &mut e {
                *offset = (script[next].0.max(at) - at) as u32;
            }
            events.push(e);
            next += 1;
        }
        let mut left = src_l[at..at + n].to_vec();
        let mut right = src_r[at..at + n].to_vec();
        let ctx = BlockCtx {
            sample_rate: rate,
            frames: n as u32,
            transport: Transport {
                playing: play,
                position_ticks: at as f64 * 0.04,
                ..Transport::default()
            },
        };
        let mut io = AudioIo {
            left: &mut left,
            right: &mut right,
            events_out: &mut unused,
        };
        assert_no_alloc::assert_no_alloc(|| f.process(&ctx, &events, &mut io));
        for i in 0..n {
            out.push(left[i]);
            out.push(right[i]);
        }
        at += n;
    }
    out
}

fn same_bits(a: &[f32], b: &[f32], what: &str) {
    assert_eq!(a.len(), b.len(), "{what}");
    for (i, (x, y)) in a.iter().zip(b).enumerate() {
        assert_eq!(x.to_bits(), y.to_bits(), "{what}: sample {i}: {x} vs {y}");
    }
}

#[test]
fn the_filter_at_its_new_defaults_renders_what_the_filter_of_dv2_did() {
    for (rate, block, play) in [
        (48_000.0, 100, false),
        (48_000.0, 128, true),
        (44_100.0, 33, true),
        (96_000.0, 1024, false),
    ] {
        let old = render(&mut old_filter::Filter::new(rate), rate, block, play, &[]);
        let new = render(&mut Filter::new(rate), rate, block, play, &[]);
        same_bits(&new, &old, &format!("{rate} Hz, blocks of {block}"));
    }
}

/// The same with every new parameter sent at its default first, as a project or a preset
/// that names them does: still the old render.
#[test]
fn the_new_parameters_sent_at_their_defaults_change_nothing() {
    let rate = 48_000.0;
    let defaults: Vec<Event> = DESCRIPTOR
        .params
        .iter()
        .filter(|p| p.id.0 > MIX)
        .map(|p| ev(p.id.0, p.default as f32))
        .collect();
    assert!(defaults.len() >= 12, "{}", defaults.len());
    let old = render(&mut old_filter::Filter::new(rate), rate, 128, true, &[]);
    let new = render(&mut Filter::new(rate), rate, 128, true, &defaults);
    same_bits(&new, &old, "new parameters at their defaults");
}
