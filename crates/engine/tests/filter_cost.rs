//! Cost of the Filter (DV-2, movement #686) in release, per stereo block of 128 frames at
//! 48 kHz (no allocation detector here: it is not built in release). `cargo test -p engine
//! --release --test filter_cost -- --ignored --nocapture`. The bound of #686: an instance that
//! moves costs under 2 % of the block's budget (2.67 ms), 53 us.

use engine::device::{AudioIo, BlockCtx, Device, EventBuffer, Transport};
use engine::devices::filter::{
    CUTOFF, DRIVE, ENV_AMOUNT, Filter, LFO_DEPTH, LFO_SHAPE, LFO_STEREO, LFO_SYNC, RESONANCE,
};
use engine::msg::ParamId;

const SR: f32 = 48_000.0;
const FRAMES: usize = 128;
const BUDGET_NS: f64 = FRAMES as f64 / SR as f64 * 1e9;

fn noise(len: usize) -> Vec<f32> {
    let mut s = 1u32;
    (0..len)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            (s as f32 / u32::MAX as f32) * 2.0 - 1.0
        })
        .collect()
}

#[test]
#[ignore]
fn cost_per_block() {
    let input = noise(FRAMES);
    let cases: [(&str, &[(u32, f32)]); 4] = [
        ("still (no movement)", &[]),
        ("LFO, both channels alike", &[(LFO_DEPTH, 2.0)]),
        (
            "LFO with stereo phase, envelope, drive",
            &[
                (LFO_DEPTH, 2.0),
                (LFO_STEREO, 90.0),
                (LFO_SHAPE, 0.0),
                (ENV_AMOUNT, 2.0),
                (DRIVE, 0.5),
            ],
        ),
        (
            "free LFO, stereo, envelope, drive, resonance",
            &[
                (LFO_SYNC, 0.0),
                (LFO_DEPTH, 3.0),
                (LFO_STEREO, 180.0),
                (ENV_AMOUNT, -2.0),
                (DRIVE, 1.0),
                (RESONANCE, 0.9),
                (CUTOFF, 400.0),
            ],
        ),
    ];
    for (label, params) in cases {
        let mut f = Filter::new(SR);
        for (id, v) in params {
            f.set_param(ParamId(*id), *v);
        }
        let mut unused = EventBuffer::try_with_capacity(1).unwrap();
        let (mut left, mut right) = (input.clone(), input.clone());
        let mut best = f64::MAX;
        for _run in 0..5 {
            let blocks = 20_000;
            let start = std::time::Instant::now();
            for n in 0..blocks {
                left.copy_from_slice(&input);
                right.copy_from_slice(&input);
                let ctx = BlockCtx {
                    sample_rate: SR,
                    frames: FRAMES as u32,
                    transport: Transport {
                        playing: true,
                        position_ticks: (n * FRAMES) as f64 * 0.04,
                        ..Transport::default()
                    },
                };
                let mut io = AudioIo {
                    left: &mut left,
                    right: &mut right,
                    events_out: &mut unused,
                };
                f.process(&ctx, &[], &mut io);
            }
            best = best.min(start.elapsed().as_nanos() as f64 / blocks as f64);
        }
        println!(
            "{label}: {best:.0} ns per block of {FRAMES} frames ({:.2} % of the block)",
            100.0 * best / BUDGET_NS
        );
        std::hint::black_box(&left);
        assert!(best < 0.02 * BUDGET_NS, "{label}: {best} ns");
    }
}
