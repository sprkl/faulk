//! Cost of the reverb (DV-4) in release, per stereo frame (no allocation detector here: it
//! is not built in release). `cargo test -p engine --release --test reverb_cost -- --ignored
//! --nocapture`.

use engine::device::{AudioIo, BlockCtx, Device, Event, EventBuffer, Transport};
use engine::devices::reverb::{DAMPING, MIX, PREDELAY, Reverb, SIZE};
use engine::msg::ParamId;

const SR: f32 = 48_000.0;

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
fn cost_per_frame() {
    let x = noise(SR as usize * 20);
    let mut best = f64::MAX;
    // (settings, whether `size` is swept from block to block: the comb gains are recomputed
    // every 16 frames while it moves, the costliest path).
    for (params, sweep) in [
        (vec![(MIX, 0.25f32)], false),
        (
            vec![(SIZE, 1.0), (DAMPING, 1.0), (MIX, 1.0), (PREDELAY, 0.2)],
            false,
        ),
        (vec![(MIX, 1.0f32)], true),
    ] {
        let mut r = Reverb::new(SR).unwrap();
        for (id, value) in &params {
            r.set_param(ParamId(*id), *value);
        }
        r.reset();
        let mut left = x.clone();
        let mut right = x.clone();
        let mut events = EventBuffer::try_with_capacity(1).unwrap();
        for _ in 0..5 {
            let start = std::time::Instant::now();
            for (n, (cl, cr)) in left.chunks_mut(256).zip(right.chunks_mut(256)).enumerate() {
                let sweep_to = Event::ParamValue {
                    offset: 0,
                    id: ParamId(SIZE),
                    value: (n % 2) as f32,
                };
                let c = BlockCtx {
                    sample_rate: SR,
                    frames: cl.len() as u32,
                    transport: Transport::default(),
                };
                let mut io = AudioIo {
                    left: cl,
                    right: cr,
                    events_out: &mut events,
                };
                let sweep_events = [sweep_to];
                let events_in: &[Event] = if sweep { &sweep_events } else { &[] };
                r.process(&c, events_in, &mut io);
            }
            best = best.min(start.elapsed().as_secs_f64() * 1e9 / x.len() as f64);
        }
        println!("{params:?}, size swept {sweep}: {best:.1} ns per frame (stereo), best of 5");
        best = f64::MAX;
    }
}
