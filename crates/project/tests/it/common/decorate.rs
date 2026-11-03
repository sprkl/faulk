//! A deterministic mix of the model's features the plain generator leaves out: names that
//! need JSON escaping, colours, loops, a module, an instrument state blob, a negative zero,
//! a transport with a loop region, extreme notes and an unusual quantization.

use super::{TrackSpec, build};
use project::*;
use std::collections::BTreeMap;

pub const NAMES: [&str; 6] = [
    "quote \" and back\\slash",
    "tab\there",
    "new\nline",
    "ctrl\u{1}\u{1f} and \u{7f}",
    "emoji \u{1F3B9} and \u{e9}",
    "slash / and </script>",
];

pub fn decorate(mut p: Project, seed: u64) -> Project {
    let pick = |salt: u64| seed.wrapping_mul(6364136223846793005).wrapping_add(salt) >> 33;
    p.name = NAMES[(pick(1) % 6) as usize].to_owned();
    p.transport.loop_region = LoopRegion::new(Tick(480), Tick(480 + 3840));
    p.transport.loop_enabled = pick(2) % 2 == 0;
    p.transport.launch_quantize = Quantize::Bars4;
    p.transport.metronome.gain_db = f64::NEG_INFINITY;
    p.master.gain_db = -0.0;
    let mut ids = IdGenerator::new(1_800_000_000_000, u128::from(seed) ^ 0xABCD);
    let mut i = 0u64;
    for track in p.tracks.values_mut() {
        i += 1;
        track.name = NAMES[((pick(i) + i) % 6) as usize].to_owned();
        track.color = Some(Rgb::new(i as u8, 255 - i as u8, 7));
        for clip in track.clips.values_mut() {
            clip.color = Some(Rgb::new(0, 0, i as u8));
            match &mut clip.content {
                ClipContent::Midi {
                    length,
                    looping,
                    notes,
                } => {
                    if length.get() > 1 {
                        *looping = Some(TickLoop {
                            start: Tick(0),
                            end: *length,
                        });
                    }
                    if length.get() >= 100 {
                        notes.insert(Note {
                            start: Tick(0),
                            pitch: 127,
                            length: Tick(1),
                            velocity: 127,
                            cents: -50,
                            chance: 0,
                            repeat: 16,
                        });
                    }
                }
                ClipContent::Audio {
                    frames, looping, ..
                } => {
                    if frames.get() > 1 {
                        *looping = Some(FrameLoop {
                            start: Frame(0),
                            end: *frames,
                        });
                    }
                }
            }
        }
        for device in track.devices.values_mut() {
            let synth =
                matches!(&device.source, DeviceSource::Builtin { id, .. } if id != CLIP_PLAYER);
            if device.role == DeviceRole::Instrument && synth {
                device.state = Some(Hash([(i as u8) ^ 0x5a; 32]));
                device.params.insert("gain".into(), -0.0);
            }
        }
        let last = track
            .devices
            .values()
            .map(|d| d.order.clone())
            .max()
            .unwrap_or_else(OrderKey::first);
        track.devices.insert(
            DeviceId::generate(&mut ids),
            Device {
                order: OrderKey::between(Some(&last), None).unwrap(),
                role: DeviceRole::Effect,
                enabled: false,
                source: DeviceSource::Module {
                    name: NAMES[0].into(),
                    hash: Hash([9; 32]),
                },
                params: BTreeMap::new(),
                state: None,
                preset: None,
            },
        );
    }
    p
}

/// A generated project with the decorations.
pub fn decorated(seed: u128, name: String, tracks: Vec<TrackSpec>, scenes: usize) -> Project {
    decorate(build(seed, name, tracks, scenes), seed as u64)
}
