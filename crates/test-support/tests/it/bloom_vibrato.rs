//! Bloom's Vibrato preset (#696, founder's feedback of 2026-11-07: "the first movement has too
//! much amplitude and length, as if playing between two notes"). The preset is loaded through
//! `preset.load` on a Bloom at its defaults while a note sounds, the way a musician steps through
//! presets (and the way `.qa/renders/preset-bloom-vibrato.wav` is made). The pitch of the output
//! is tracked period by period (rising zero crossings of a sine note) and the curve, in cents
//! from the note, is written next to the render under `.qa/696/`. The first oscillation after
//! the load must swing as wide and last as long as the next ones.

use commands::preset::PresetRef;
use commands::testing::{context, fixture};
use commands::{Command, Output, device, live, preset};
use project::preset::PresetLocation;
use test_support::{Render, Session, parity};

const NOTE_HZ: f64 = 440.0;
/// When the preset is loaded, in seconds.
const LOAD_AT: f64 = 0.3;

fn set(device: &project::DeviceId, param: &str, value: f64) -> Command {
    Command::Device(device::Command::SetParam {
        device: device.clone(),
        param: param.into(),
        value,
    })
}

/// A sine A4 held through a Bloom at its defaults, the Vibrato preset loaded at `LOAD_AT`.
fn render() -> Render {
    let f = fixture();
    let mut s = Session::from_state(f.state.clone(), context());
    for (param, value) in [
        ("waveform", 0.0),
        ("attack", 0.005),
        ("sustain", 1.0),
        ("gain", 0.5),
    ] {
        s.run(set(&f.device, param, value));
    }
    let out = s.exec(Command::Device(device::Command::Insert {
        track: Some(f.track.clone()),
        master: false,
        kind: "faulk.bloom".into(),
        index: 0,
    }));
    let Output::Device(device::Output::Insert { device: fx }) = out else {
        panic!("{out:?}")
    };
    s.run(Command::Live(live::Command::NoteOn {
        track: f.track.clone(),
        note: 69,
        velocity: 100,
    }));
    s.render_secs(LOAD_AT as f32);
    s.run(Command::Preset(preset::Command::Load {
        device: fx,
        preset: PresetRef {
            location: PresetLocation::Factory,
            id: "faulk.bloom/vibrato".into(),
        },
    }));
    s.render_secs(1.7);
    parity(s.scenario())
}

/// The pitch of each period, `(time of its middle in seconds, cents from the note)`.
fn pitch(x: &[f32], rate: f64) -> Vec<(f64, f64)> {
    let ups: Vec<f64> = x
        .windows(2)
        .enumerate()
        .filter(|(_, w)| w[0] < 0.0 && w[1] >= 0.0)
        .map(|(i, w)| i as f64 + f64::from(w[0] / (w[0] - w[1])))
        .collect();
    ups.windows(2)
        .map(|w| {
            let hz = rate / (w[1] - w[0]);
            ((w[0] + w[1]) / 2.0 / rate, 1200.0 * (hz / NOTE_HZ).log2())
        })
        .collect()
}

/// The swing (highest minus lowest cents) and the length, in seconds, of each LFO cycle after
/// `from`, the cycles cut at the upward crossings of the curve's middle.
fn cycles(curve: &[(f64, f64)], from: f64) -> Vec<(f64, f64, f64)> {
    let after: Vec<(f64, f64)> = curve.iter().copied().filter(|(t, _)| *t >= from).collect();
    let mid = {
        let tail = &after[after.len() / 2..];
        let (lo, hi) = tail.iter().fold((f64::MAX, f64::MIN), |(l, h), (_, c)| {
            (l.min(*c), h.max(*c))
        });
        (lo + hi) / 2.0
    };
    let crossings: Vec<usize> = (1..after.len())
        .filter(|&i| after[i - 1].1 < mid && after[i].1 >= mid)
        .collect();
    crossings
        .windows(2)
        .map(|w| {
            let part = &after[w[0]..w[1]];
            let (lo, hi) = part.iter().fold((f64::MAX, f64::MIN), |(l, h), (_, c)| {
                (l.min(*c), h.max(*c))
            });
            (after[w[0]].0, hi - lo, after[w[1]].0 - after[w[0]].0)
        })
        .collect()
}

pub fn write_curve(name: &str, render: &Render, curve: &[(f64, f64)]) {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.qa/696");
    std::fs::create_dir_all(&dir).expect("the folder is created");
    std::fs::write(dir.join(format!("{name}.wav")), render.wav()).expect("the WAV is written");
    let mut csv = String::from("seconds,cents\n");
    for (t, c) in curve {
        csv.push_str(&format!("{t:.5},{c:.2}\n"));
    }
    std::fs::write(dir.join(format!("{name}-pitch.csv")), csv).expect("the curve is written");
}

#[test]
fn the_vibrato_preset_swings_from_its_first_oscillation_as_it_goes_on() {
    let r = render();
    r.assert_clean();
    let curve = pitch(&r.left, f64::from(r.sample_rate));
    let name = std::env::var("FAULK_BLOOM_VIBRATO_NAME").unwrap_or("bloom-vibrato".into());
    write_curve(&name, &r, &curve);
    // Before the load: the default chorus (0.8 Hz, 40 %), a slow, small drift.
    let before: Vec<f64> = curve
        .iter()
        .filter(|(t, _)| (0.05..LOAD_AT).contains(t))
        .map(|(_, c)| *c)
        .collect();
    let after_load: Vec<(f64, f64)> = curve
        .iter()
        .copied()
        .filter(|(t, _)| *t >= LOAD_AT)
        .collect();
    let extreme = after_load.iter().fold(0f64, |m, (_, c)| m.max(c.abs()));
    let cyc = cycles(&curve, LOAD_AT);
    println!(
        "before the load: {:+.1} .. {:+.1} cents",
        before.iter().cloned().fold(f64::MAX, f64::min),
        before.iter().cloned().fold(f64::MIN, f64::max)
    );
    println!("after the load: largest deviation {extreme:.1} cents");
    for (t, swing, len) in &cyc {
        println!(
            "cycle at {t:.3} s: swing {swing:.1} cents, length {:.1} ms",
            len * 1e3
        );
    }
    // The first oscillation after the load is like the next ones, and the preset stays in the
    // range of a musical vibrato.
    let (first, rest) = cyc.split_first().expect("cycles after the load");
    let swing = rest.iter().map(|c| c.1).sum::<f64>() / rest.len() as f64;
    let length = rest.iter().map(|c| c.2).sum::<f64>() / rest.len() as f64;
    assert!(
        (first.1 - swing).abs() < 0.15 * swing,
        "first swing {} vs {swing}",
        first.1
    );
    assert!(
        (first.2 - length).abs() < 0.15 * length,
        "first cycle {} s vs {length}",
        first.2
    );
    assert!(extreme < 50.0, "{extreme} cents");
    // About +/-30 cents (a musical vibrato), its middle never more than the centre's drift
    // (5 cents) away from the note.
    assert!((50.0..70.0).contains(&swing), "{swing}");
    let middles: Vec<f64> = after_load
        .iter()
        .map(|(_, c)| *c)
        .collect::<Vec<_>>()
        .chunks(80)
        .filter(|c| c.len() == 80)
        .map(|c| {
            let (lo, hi) = c
                .iter()
                .fold((f64::MAX, f64::MIN), |(l, h), x| (l.min(*x), h.max(*x)));
            (lo + hi) / 2.0
        })
        .collect();
    println!("middle of each 80 periods: {middles:.1?}");
    assert!(middles.iter().all(|m| m.abs() < 8.0), "{middles:?}");
}
