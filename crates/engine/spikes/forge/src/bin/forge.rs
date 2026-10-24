//! The spike's driver.
//!
//! - `forge render <dir> [<raw dir>]`: renders the three sketches at 48 kHz (2×), matched
//!   in loudness, to `<dir>/forge-sketch-{a,b,c}.wav`, and the raw renders (`.f32`, before
//!   matching, in `<raw dir>`, else `<dir>`) for the native/WASM comparison.
//! - `forge bench`: CPU cost natively (run in release).
//! - `forge aliasing`: aliasing of the saw at 4.1 kHz and of a driven voice.
//! - `forge compare <dir>`: native raw renders against the WASM ones (`-wasm.f32`).

use std::time::Instant;

use forge_spike::analysis::{SawKind, alias_report, bad_samples, peak, rms, saw, wav_24};
use forge_spike::bench;
use forge_spike::song::{Performance, SKETCHES};
use forge_spike::synth::{Oversample, Synth};
use forge_spike::voice::{FilterKind, Patch};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("render") => render(args.get(2).expect("output folder"), args.get(3)),
        Some("bench") => cpu(),
        Some("aliasing") => aliasing(),
        Some("compare") => compare(args.get(2).expect("folder of the raw renders")),
        _ => eprintln!("usage: forge render <dir> | bench | aliasing | compare <dir>"),
    }
}

fn write_raw(path: &std::path::Path, l: &[f32], r: &[f32]) {
    let mut bytes = Vec::with_capacity(l.len() * 8);
    for (a, b) in l.iter().zip(r) {
        bytes.extend_from_slice(&a.to_le_bytes());
        bytes.extend_from_slice(&b.to_le_bytes());
    }
    std::fs::write(path, bytes).expect("write raw");
}

fn read_raw(path: &std::path::Path) -> Vec<f32> {
    let bytes = std::fs::read(path).expect("read raw");
    bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| f32::from_le_bytes(*c))
        .collect()
}

fn render(dir: &str, raw_dir: Option<&String>) {
    let dir = std::path::Path::new(dir);
    std::fs::create_dir_all(dir).expect("output folder");
    let raw_dir = raw_dir.map_or(dir, |d| std::path::Path::new(d));
    std::fs::create_dir_all(raw_dir).expect("raw folder");
    let target_rms = 10f32.powf(-18.0 / 20.0);
    let mut renders = Vec::new();
    for sketch in &SKETCHES {
        let start = Instant::now();
        let (l, r) = Performance::new(sketch, bench::FS, Oversample::X2, 349).render();
        let took = start.elapsed().as_secs_f32();
        let (nan_l, den_l) = bad_samples(&l);
        let (nan_r, den_r) = bad_samples(&r);
        write_raw(
            &raw_dir.join(format!("forge-sketch-{}.f32", sketch.name)),
            &l,
            &r,
        );
        let level = rms(&l).hypot(rms(&r)) / std::f32::consts::SQRT_2;
        println!(
            "sketch {}: {:.1} s rendered in {:.2} s, raw peak {:.3}, raw rms {:.4}, non-finite {}, denormal {}",
            sketch.name,
            l.len() as f32 / bench::FS,
            took,
            peak(&l).max(peak(&r)),
            level,
            nan_l + nan_r,
            den_l + den_r
        );
        renders.push((sketch.name, l, r, level));
    }
    // Same loudness (RMS) for a fair comparison; one common trim if a peak would clip.
    let mut gains: Vec<f32> = renders.iter().map(|x| target_rms / x.3).collect();
    let worst = renders
        .iter()
        .zip(&gains)
        .map(|(x, g)| peak(&x.1).max(peak(&x.2)) * g)
        .fold(0.0, f32::max);
    if worst > 0.95 {
        for g in &mut gains {
            *g *= 0.95 / worst;
        }
    }
    for ((name, l, r, _), g) in renders.iter().zip(&gains) {
        let l: Vec<f32> = l.iter().map(|v| v * g).collect();
        let r: Vec<f32> = r.iter().map(|v| v * g).collect();
        let path = dir.join(format!("forge-sketch-{name}.wav"));
        std::fs::write(&path, wav_24(&l, &r, bench::FS as u32)).expect("write wav");
        println!(
            "{}: gain {:.2} dB, peak {:.2} dBFS, rms {:.2} dBFS",
            path.display(),
            20.0 * g.log10(),
            20.0 * peak(&l).max(peak(&r)).log10(),
            20.0 * (rms(&l).hypot(rms(&r)) / std::f32::consts::SQRT_2).log10()
        );
    }
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

/// Percent of one core: render time over audio time, median of `runs`.
fn measure(variant: u32, voices: usize, seconds: f32, runs: usize) -> f64 {
    let mut times = Vec::new();
    for _ in 0..runs {
        let mut synth: Synth = bench::prepare(variant, voices);
        let start = Instant::now();
        let sum = bench::run(&mut synth, seconds);
        let took = start.elapsed().as_secs_f64();
        std::hint::black_box(sum);
        times.push(took / seconds as f64 * 100.0);
    }
    median(times)
}

fn cpu() {
    let names = ["ladder 2x", "svf 2x", "ladder 1x", "svf 1x"];
    let seconds = 10.0;
    println!("variant, voices, % of a core (median of 7 runs of {seconds} s at 48 kHz)");
    for variant in 0..4u32 {
        let empty = measure(variant, 0, seconds, 7);
        let one = measure(variant, 1, seconds, 7);
        let sixteen = measure(variant, 16, seconds, 7);
        println!(
            "{:9}: 0 voice {:.3} %, 1 voice {:.3} %, 16 voices {:.3} %, per voice (16-0)/16 {:.3} %",
            names[variant as usize],
            empty,
            one,
            sixteen,
            (sixteen - empty) / 16.0
        );
    }
}

fn aliasing() {
    let fs = 48_000.0;
    let hz = 4_100.0;
    let frames = 1 << 16;
    println!(
        "saw at {hz} Hz, fs {fs}, {frames} frames (4 kHz exactly would fold every alias onto a harmonic: 48000/4000 = 12)"
    );
    for (kind, os, name) in [
        (SawKind::Naive, Oversample::X1, "naive 1x"),
        (SawKind::PolyBlep, Oversample::X1, "PolyBLEP 1x"),
        (SawKind::Naive, Oversample::X2, "naive 2x"),
        (SawKind::PolyBlep, Oversample::X2, "PolyBLEP 2x"),
    ] {
        let x = saw(kind, os, hz, fs, frames);
        let r = alias_report(&x, fs as f64, hz as f64, 19_000.0);
        println!(
            "{name:12}: worst alias {:.1} dBc at {:.0} Hz; below 19 kHz {:.1} dBc at {:.0} Hz; alias/harmonics {:.1} dB",
            r.worst_dbc,
            r.worst_hz,
            r.worst_audible_dbc,
            r.worst_audible_hz,
            r.alias_to_harmonics_db
        );
    }
    // A whole voice: saw through the ladder wide open, clean then driven hard (the
    // saturation's own aliasing), at 1x and 2x, at C6 (1047 Hz) and C8 (4186 Hz, the
    // nearest whole key to 4.1 kHz).
    for (key, drive, os, name) in [
        (84u8, 1.5f32, Oversample::X1, "voice 1x"),
        (84, 1.5, Oversample::X2, "voice 2x"),
        (84, 6.0, Oversample::X1, "voice 1x"),
        (84, 6.0, Oversample::X2, "voice 2x"),
        (108, 1.5, Oversample::X1, "voice 1x"),
        (108, 1.5, Oversample::X2, "voice 2x"),
        (108, 6.0, Oversample::X1, "voice 1x"),
        (108, 6.0, Oversample::X2, "voice 2x"),
    ] {
        let patch = Patch {
            level_a: 0.8,
            filter: FilterKind::Ladder,
            cutoff_hz: 20_000.0,
            resonance: 0.0,
            key_track: 0.0,
            drive,
            out_drive: if drive > 2.0 { 2.0 } else { 1.0 },
            age: 0.0,
            level: 1.0,
            amp: forge_spike::env::Adsr {
                attack: 0.001,
                decay: 0.1,
                sustain: 1.0,
                release: 0.1,
            },
            ..Patch::INIT
        };
        let mut synth = Synth::new(fs, os, patch, forge_spike::chorus::ChorusSettings::OFF, 1);
        synth.note_on(key, 1.0);
        let mut l = vec![0.0f32; frames + 4096];
        let mut r = vec![0.0f32; frames + 4096];
        for (lc, rc) in l.chunks_mut(128).zip(r.chunks_mut(128)) {
            synth.render(lc, rc);
        }
        let f0 = forge_spike::math::key_hz(key as f32) as f64;
        let rep = alias_report(&l[4096..], fs as f64, f0, 19_000.0);
        println!(
            "{name:12}: ({f0:.0} Hz, drive {drive}) worst alias {:.1} dBc at {:.0} Hz; below 19 kHz {:.1} dBc at {:.0} Hz; alias/harmonics {:.1} dB",
            rep.worst_dbc,
            rep.worst_hz,
            rep.worst_audible_dbc,
            rep.worst_audible_hz,
            rep.alias_to_harmonics_db
        );
    }
}

fn compare(dir: &str) {
    let dir = std::path::Path::new(dir);
    for sketch in &SKETCHES {
        let native = read_raw(&dir.join(format!("forge-sketch-{}.f32", sketch.name)));
        let wasm = read_raw(&dir.join(format!("forge-sketch-{}-wasm.f32", sketch.name)));
        assert_eq!(native.len(), wasm.len(), "lengths differ");
        let (mut worst, mut at) = (0.0f32, 0);
        for (i, (a, b)) in native.iter().zip(&wasm).enumerate() {
            let d = (a - b).abs();
            if d > worst {
                worst = d;
                at = i;
            }
        }
        println!(
            "sketch {}: max |native - wasm| = {:.3e} at frame {} ({:.2} s), raw peak {:.3}",
            sketch.name,
            worst,
            at / 2,
            (at / 2) as f32 / bench::FS,
            peak(&native)
        );
    }
}
