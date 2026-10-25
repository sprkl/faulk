//! Issue #353: the Reverb's default (Glide) pre-delay renders the very same samples as before
//! the `time_change` setting existed. The fingerprints below were taken on `develop` at
//! 6b2023c, the build that preceded it (FNV-1a over the bits of every output sample), by this
//! very test with the three values at zero.

use engine::device::{AudioIo, BlockCtx, Device, Event, EventBuffer, Transport};
use engine::devices::reverb::{DAMPING, MIX, PREDELAY, Reverb, SIZE};
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

fn ev(offset: u32, id: u32, value: f32) -> Event {
    Event::ParamValue {
        offset,
        id: ParamId(id),
        value,
    }
}

fn fingerprint(rate: f32) -> u64 {
    let mut r = Reverb::new(rate).unwrap();
    r.reset();
    let total = (rate * 6.0) as usize;
    let (src_l, src_r) = (noise(total, 7), noise(total, 99));
    // Large moves both ways, small moves, a change every few frames, 0 and the maximum.
    let mut script: Vec<(usize, Event)> = vec![
        (0, ev(0, MIX, 0.6)),
        (0, ev(0, SIZE, 0.3)),
        (0, ev(0, DAMPING, 0.2)),
    ];
    let values = [0.2, 0.0, 0.123, 0.1234, 0.05, 0.2, 0.0021, 0.0, 0.07];
    for (i, v) in values.iter().enumerate() {
        script.push((5_000 + i * 33_001 % 250_000, ev(0, PREDELAY, *v)));
    }
    // Upward glides from a start off the quarter-frame grid, across several powers of two
    // (the delay in use is rounded to the grid of its magnitude at every step).
    for (at, v) in [
        (0, 0.00213),
        (150_000, 0.0017),
        (160_000, 0.15),
        (200_000, 0.00131),
        (210_000, 0.2),
    ] {
        script.push((at, ev(0, PREDELAY, v)));
    }
    for k in 0..40 {
        script.push((
            100_000 + k * 37,
            ev(0, PREDELAY, 0.001 * ((k * 7919) % 200) as f32),
        ));
    }
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    let mut unused = EventBuffer::try_with_capacity(1).unwrap();
    let mut start = 0;
    let sizes = [64usize, 100, 128, 1, 513];
    let mut n = 0;
    while start < total {
        let end = (start + sizes[n % sizes.len()]).min(total);
        n += 1;
        let mut left = src_l[start..end].to_vec();
        let mut right = src_r[start..end].to_vec();
        let mut local: Vec<Event> = script
            .iter()
            .filter(|(at, _)| (start..end).contains(at))
            .map(|(at, e)| e.at((at - start) as u32))
            .collect();
        local.sort_by_key(|e| e.offset());
        let ctx = BlockCtx {
            sample_rate: rate,
            frames: (end - start) as u32,
            transport: Transport::default(),
        };
        let mut io = AudioIo {
            left: &mut left,
            right: &mut right,
            events_out: &mut unused,
        };
        r.process(&ctx, &local, &mut io);
        for s in left.iter().chain(right.iter()) {
            for b in s.to_bits().to_le_bytes() {
                hash = (hash ^ u64::from(b)).wrapping_mul(0x100_0000_01b3);
            }
        }
        start = end;
    }
    hash
}

#[test]
fn the_default_pre_delay_renders_as_before_time_change_existed() {
    let got = [
        fingerprint(48_000.0),
        fingerprint(44_100.0),
        fingerprint(96_000.0),
    ];
    println!("fingerprints {got:#018x?}");
    assert_eq!(
        got,
        [
            0x209b_dd7a_30a6_17a8,
            0x71a7_b20e_5137_f3fa,
            0xa79c_f569_d0ba_6783
        ]
    );
}
