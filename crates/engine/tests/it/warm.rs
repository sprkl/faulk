//! Warm-up of the apply path (#355): it exercises every message, leaves the real channel
//! untouched and makes no sound anywhere but in its own private host.

use engine::host::{Host, Prepare, channel};
use engine::msg::{ControlMsg as M, DeviceSlot, Tag, TrackKind, TrackSlot, encode_stream};
use engine::time::Tick;
use engine::{OSC_SYNTH, warm};

const RATE: f32 = 48_000.0;

#[test]
fn every_message_kind_is_sent_at_least_once() {
    let sent = warm::tags();
    let missing: Vec<Tag> = Tag::ALL
        .iter()
        .copied()
        .filter(|t| !sent.contains(t))
        .collect();
    assert!(missing.is_empty(), "not warmed: {missing:?}");
}

#[test]
fn the_throwaway_engine_applies_everything_and_rejects_nothing() {
    let report = warm::warm_up(RATE).unwrap();
    assert_eq!(report.rejected, 0, "{report:?}");
    assert_eq!(report.skipped, 0, "{report:?}");
    // Every message but the live ones (notes) goes through the rt ring and is applied.
    let live = warm::tags().iter().filter(|t| t.is_live()).count() as u64;
    assert_eq!(report.applied, u64::from(report.sent) - live, "{report:?}");
    assert!(report.blocks >= 60, "{report:?}");
    // The devices sounded in the private host (and nowhere else).
    assert!(report.peak > 0.01, "{report:?}");
    // The transport phase reached the loop's end and wrapped, and a recording counted in.
    assert!(report.loop_wraps >= 1, "{report:?}");
    assert!(report.lowest_tick < 0, "{report:?}");
}

/// The warm-up inserts every device of the registry, MIDI effects included (#219: at the head
/// of the first track's chain, so the warm-up's notes run through them).
#[test]
fn every_device_of_the_registry_is_warmed() {
    use engine::devices::REGISTRY;
    let inserted: Vec<_> = warm::messages()
        .iter()
        .flat_map(|p| &p.messages)
        .filter_map(|m| match m {
            M::DeviceInsert { kind, .. } => Some(*kind),
            _ => None,
        })
        .collect();
    for builtin in REGISTRY {
        assert!(
            inserted.contains(&builtin.descriptor.kind_id),
            "{} is not warmed",
            builtin.descriptor.kind
        );
    }
}

/// A session: a track with the synth and a held note. Renders blocks of 128 frames and
/// returns every sample with the audio side's counters; `warm_between` runs a warm-up in the
/// middle.
fn session(warm_between: bool) -> (Vec<u32>, u64, u32) {
    let (mut prepare, mut host) = channel(RATE).unwrap();
    let bytes = encode_stream(
        1,
        0,
        &[
            M::Reset { epoch: 1 },
            M::TrackCreate {
                track: TrackSlot(0),
                kind: TrackKind::Midi,
            },
            M::DeviceInsert {
                track: TrackSlot(0),
                device: DeviceSlot(0),
                kind: OSC_SYNTH.kind_id,
                state: Vec::new(),
            },
            M::Play { from: Tick(0) },
            M::NoteOn {
                track: TrackSlot(0),
                pitch: 60,
                velocity: 100,
                at: None,
            },
        ],
    )
    .unwrap();
    let submit = |prepare: &mut Prepare| prepare.submit(&bytes).unwrap();
    submit(&mut prepare);
    let mut out = Vec::new();
    let render = |host: &mut Host, out: &mut Vec<u32>| {
        host.render(128);
        out.extend(host.left()[..128].iter().map(|s| s.to_bits()));
        out.extend(host.right()[..128].iter().map(|s| s.to_bits()));
    };
    render(&mut host, &mut out);
    if warm_between {
        warm::warm_up(RATE).unwrap();
    }
    for _ in 0..8 {
        render(&mut host, &mut out);
    }
    (out, host.engine().applied(), host.engine().skipped())
}

#[test]
fn the_real_state_is_unchanged_by_a_warm_up() {
    let (plain, applied, skipped) = session(false);
    let (warmed, applied_w, skipped_w) = session(true);
    assert!(
        plain.iter().any(|s| f32::from_bits(*s).abs() > 1e-4),
        "the reference must sound"
    );
    // Bit for bit: the same samples, the same counters.
    assert_eq!(plain, warmed);
    assert_eq!((applied, skipped), (applied_w, skipped_w));
}

#[test]
fn a_warm_up_can_run_twice() {
    let first = warm::warm_up(RATE).unwrap();
    assert_eq!(warm::warm_up(RATE).unwrap(), first);
}
