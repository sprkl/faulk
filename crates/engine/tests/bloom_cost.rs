//! Cost of Bloom (#696) in release, per block of 128 stereo frames at 48 kHz (no allocation
//! detector here: it is not built in release). `cargo test -p engine --release --test
//! bloom_cost -- --ignored --nocapture`.

use engine::device::{AudioIo, BlockCtx, Device, EventBuffer, Transport};
use engine::devices::bloom::{Bloom, DEPTH, ENSEMBLE, FEEDBACK, MIX, MODE, SYNC, TONE};
use engine::msg::ParamId;
use engine::time::Tempo;

const SR: f32 = 48_000.0;
const BLOCK: usize = 128;

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
    let x = noise(SR as usize * 20);
    let blocks = x.len() / BLOCK;
    // The default chorus; the heaviest case: ensemble (three voices per channel), synced,
    // feedback, the tone ramping from block to block (its coefficient recomputed every frame).
    for (name, params, ramp_tone) in [
        ("chorus (defaults)", vec![], false),
        (
            "ensemble, synced, feedback, tone ramping",
            vec![
                (MODE, ENSEMBLE),
                (SYNC, 1.0),
                (DEPTH, 100.0),
                (FEEDBACK, 60.0),
                (MIX, 1.0),
            ],
            true,
        ),
    ] {
        let mut b = Bloom::new(SR).unwrap();
        for (id, value) in &params {
            b.set_param(ParamId(*id), *value);
        }
        b.reset();
        let mut left = x.clone();
        let mut right = x.clone();
        let mut events = EventBuffer::try_with_capacity(1).unwrap();
        let mut best = f64::MAX;
        for _ in 0..5 {
            let start = std::time::Instant::now();
            for (n, (cl, cr)) in left
                .chunks_mut(BLOCK)
                .zip(right.chunks_mut(BLOCK))
                .enumerate()
            {
                if ramp_tone {
                    b.set_param(ParamId(TONE), if n % 2 == 0 { 2_000.0 } else { 20.0 });
                }
                let c = BlockCtx {
                    sample_rate: SR,
                    frames: cl.len() as u32,
                    transport: Transport {
                        playing: true,
                        tempo: Tempo::new(120.0).unwrap(),
                        position_ticks: (n * BLOCK) as f64 * 0.04,
                        ..Transport::default()
                    },
                };
                let mut io = AudioIo {
                    left: cl,
                    right: cr,
                    events_out: &mut events,
                };
                b.process(&c, &[], &mut io);
            }
            best = best.min(start.elapsed().as_secs_f64() * 1e6 / blocks as f64);
        }
        let budget = BLOCK as f64 / f64::from(SR) * 1e6;
        println!(
            "{name}: {best:.2} us per block of {BLOCK} ({:.2} % of the block's {budget:.0} us), best of 5",
            100.0 * best / budget
        );
    }
}
