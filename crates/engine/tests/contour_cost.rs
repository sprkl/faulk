//! Cost of the equaliser Contour (#156) in release (no allocation detector here: it is not built
//! in release): the device per stereo frame, at rest and with every band's frequency moving; the
//! audio side's copy of 4 analysis windows per report (`Host::write_feedback`); the prepare side's
//! analysis of 4 windows (`Prepare::analyse`, on the helper's control thread and the web prepare
//! Worker). `cargo test -p engine --release --test contour_cost -- --ignored --nocapture`.

use engine::analysis::{
    Analysis, MAX_SPECTRA, SPECTRUM_REGION_WORDS, Source, WINDOW_REGION_WORDS, WindowWriter,
    try_region,
};
use engine::device::{AudioIo, BlockCtx, Device, Event, EventBuffer, Transport};
use engine::devices::contour::{BANDS, BandParam, Contour, band_param};
use engine::msg::ParamId;
use std::time::Instant;

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
fn cost() {
    let x = noise(SR as usize * 10);
    for moving in [false, true] {
        let mut best = f64::MAX;
        for _ in 0..5 {
            let mut c = Contour::new(SR);
            for b in 0..BANDS {
                c.set_param(ParamId(band_param(b, BandParam::On).unwrap()), 1.0);
            }
            c.reset();
            let mut unused = EventBuffer::try_with_capacity(1).unwrap();
            let (mut l, mut r) = (vec![0.0; 256], vec![0.0; 256]);
            let start = Instant::now();
            for (i, chunk) in x.chunks(256).enumerate() {
                l[..chunk.len()].copy_from_slice(chunk);
                r[..chunk.len()].copy_from_slice(chunk);
                let events: Vec<Event> = if moving {
                    (0..BANDS)
                        .map(|b| Event::ParamValue {
                            offset: 0,
                            id: ParamId(band_param(b, BandParam::Freq).unwrap()),
                            value: 100.0 + (i % 50) as f32 * 100.0,
                        })
                        .collect()
                } else {
                    Vec::new()
                };
                let ctx = BlockCtx {
                    sample_rate: SR,
                    frames: chunk.len() as u32,
                    transport: Transport::default(),
                };
                c.process(
                    &ctx,
                    &events,
                    &mut AudioIo {
                        left: &mut l[..chunk.len()],
                        right: &mut r[..chunk.len()],
                        events_out: &mut unused,
                    },
                );
            }
            let ns = start.elapsed().as_secs_f64() * 1e9 / x.len() as f64;
            best = best.min(ns);
            std::hint::black_box(&l);
        }
        println!(
            "Contour, every band on, {}: {best:.1} ns per stereo frame ({:.2} % of a core at 48 kHz)",
            if moving {
                "every frequency moving"
            } else {
                "at rest"
            },
            best * SR as f64 / 1e7
        );
    }

    // The analysis: 4 windows of 2048 frames copied (the audio side), then analysed (prepare).
    let windows = try_region(WINDOW_REGION_WORDS).unwrap();
    let spectra = try_region(SPECTRUM_REGION_WORDS).unwrap();
    let mut writer = WindowWriter::default();
    let mut analysis = Analysis::try_new().unwrap();
    let frames = noise(2048);
    let (mut copy, mut analyse) = (f64::MAX, f64::MAX);
    for _ in 0..50 {
        let start = Instant::now();
        writer.begin(&windows);
        for d in 0..MAX_SPECTRA as u32 {
            writer.push(
                &windows,
                Source {
                    track: 0,
                    device: d,
                },
                SR,
                &frames,
            );
        }
        writer.end(&windows);
        copy = copy.min(start.elapsed().as_secs_f64() * 1e6);
        let start = Instant::now();
        assert!(analysis.run(&windows, &spectra));
        analyse = analyse.min(start.elapsed().as_secs_f64() * 1e6);
    }
    println!(
        "analysis of {MAX_SPECTRA} windows of 2048 frames: copy on the audio side {copy:.1} µs per report, \
         analysis off the audio path {analyse:.1} µs per report (about 30 per second)"
    );

    // The whole report on the audio side, the walks of the chains and the copies included: 16
    // tracks of a synth and 4 Contours each (64 devices to walk, 4 windows copied per report).
    let (mut prepare, mut host) = engine::host::channel(SR).unwrap();
    let mut msgs = vec![engine::msg::ControlMsg::Reset { epoch: 1 }];
    for t in 0..16u32 {
        let track = engine::msg::TrackSlot(t);
        msgs.push(engine::msg::ControlMsg::TrackCreate {
            track,
            kind: engine::msg::TrackKind::Midi,
        });
        msgs.push(engine::msg::ControlMsg::DeviceInsert {
            track,
            device: engine::msg::DeviceSlot(0),
            kind: engine::OSC_SYNTH.kind_id,
            state: Vec::new(),
        });
        for d in 1..=4u16 {
            msgs.push(engine::msg::ControlMsg::DeviceInsert {
                track,
                device: engine::msg::DeviceSlot(d),
                kind: engine::devices::contour::DESCRIPTOR.kind_id,
                state: Vec::new(),
            });
        }
    }
    let stream = engine::msg::encode_stream(1, 0, &msgs).unwrap();
    prepare.submit(&stream).unwrap();
    for _ in 0..40 {
        host.render(128);
        prepare.flush();
    }
    let (mut best, mut worst) = (f64::MAX, 0f64);
    for _ in 0..200 {
        host.render(128);
        let start = Instant::now();
        host.write_feedback();
        let us = start.elapsed().as_secs_f64() * 1e6;
        best = best.min(us);
        worst = worst.max(us);
        prepare.flush();
    }
    println!(
        "a report on the audio side (Host::write_feedback, its walks and copies included), 16 tracks \
         of a synth and 4 Contours: best {best:.1} µs, worst {worst:.1} µs of 200"
    );
}
