//! Key changes (#953) on the audio side: the table refuses what the binary search cannot read,
//! finds the scale in force, and the per-block switches give every event tick the key of its
//! own tick whatever the block size, across a key change, a loop wrap into another key and a
//! seek, with no allocation.

use assert_no_alloc::assert_no_alloc;
use engine::device::{PitchSet, ScaleSwitches, Transport as DeviceTransport};
use engine::engine::State;
use engine::keymap::{KeyMap, MAX_KEY_CHANGES};
use engine::time::{LoopRegion, SampleRate, Tick};
use engine::transport::{Segment, Transport};

const C_MAJOR: u16 = 0b1010_1011_0101;
const D_MAJOR: u16 = 0b1011_0101_0101 & 0x0FFF;
const START: PitchSet = PitchSet::from_bits(C_MAJOR);

fn map(changes: &[(i64, u16)]) -> KeyMap {
    KeyMap::new(changes).expect("a sorted table")
}

#[test]
fn a_table_refuses_what_is_not_sorted_positive_or_small_enough() {
    assert!(KeyMap::new(&[]).is_some());
    assert!(KeyMap::new(&[(0, 1)]).is_none(), "tick 0 is the start's");
    assert!(KeyMap::new(&[(-5, 1)]).is_none());
    assert!(KeyMap::new(&[(10, 1), (10, 2)]).is_none(), "twice");
    assert!(KeyMap::new(&[(20, 1), (10, 2)]).is_none(), "unordered");
    let many: Vec<(i64, u16)> = (1..=MAX_KEY_CHANGES as i64 + 1).map(|t| (t, 1)).collect();
    assert!(KeyMap::new(&many).is_none());
    assert!(KeyMap::new(&many[..MAX_KEY_CHANGES]).is_some());
    // Bits above the twelfth are dropped.
    assert_eq!(
        KeyMap::new(&[(5, 0xFFFF)]).unwrap().changes().next(),
        Some((5, PitchSet::from_bits(0x0FFF)))
    );
}

#[test]
fn the_scale_in_force_is_the_last_change_at_or_before() {
    let m = map(&[(3840, D_MAJOR), (7680, 0)]);
    assert_eq!(m.scale_at(START, 0.0), START);
    assert_eq!(m.scale_at(START, 3839.9), START);
    assert_eq!(m.scale_at(START, 3840.0), PitchSet::from_bits(D_MAJOR));
    assert_eq!(m.scale_at(START, 7679.0), PitchSet::from_bits(D_MAJOR));
    // A change to "no scale" is the empty set, not the start.
    assert_eq!(m.scale_at(START, 7680.0), PitchSet::EMPTY);
    assert_eq!(m.scale_at(START, 1e18), PitchSet::EMPTY);
    // A position that is not a number, or before everything, is the start.
    assert_eq!(m.scale_at(START, f64::NAN), START);
    assert_eq!(m.scale_at(START, -1.0), START);
    assert_eq!(KeyMap::new(&[]).unwrap().scale_at(START, 5e9), START);
}

#[test]
fn the_switches_keep_a_fixed_capacity_and_answer_by_frame() {
    let mut s = ScaleSwitches::NONE;
    assert!(s.is_empty());
    assert_eq!(s.at(100), None);
    for i in 0..ScaleSwitches::CAPACITY as u32 + 3 {
        let fitted = s.push(i * 10, PitchSet::from_bits(1 << (i % 12)));
        assert_eq!(fitted, (i as usize) < ScaleSwitches::CAPACITY);
    }
    assert_eq!(s.len(), ScaleSwitches::CAPACITY);
    assert_eq!(s.at(0), Some(PitchSet::from_bits(1)));
    assert_eq!(s.at(9), Some(PitchSet::from_bits(1)));
    assert_eq!(s.at(10), Some(PitchSet::from_bits(2)));
    let t = DeviceTransport {
        scale: START,
        scale_switches: s,
        ..DeviceTransport::default()
    };
    assert_eq!(t.scale_at(0), PitchSet::from_bits(1));
    let none = DeviceTransport {
        scale: START,
        ..DeviceTransport::default()
    };
    assert_eq!(none.scale_at(500), START);
}

/// Renders blocks of `size` frames and checks that the key an event at each tick would see (the
/// block's scale and its switches, at the frame the tick lands on) is the key at that tick.
fn check(
    rate: u32,
    size: usize,
    blocks: usize,
    keys: &KeyMap,
    setup: impl Fn(&mut State, &mut Transport),
) -> usize {
    let mut t = Transport::new(SampleRate::new(rate).unwrap());
    let mut s = State::try_new().unwrap();
    s.playback.playing = true;
    s.playback.cue = Some(Tick(0));
    setup(&mut s, &mut t);
    let mut switched = 0;
    for _ in 0..blocks {
        t.begin_block(&mut s, size);
        let mut segments: Vec<Segment> = Vec::new();
        while let Some(seg) = t.next_segment() {
            segments.push(seg);
        }
        let position = segments.first().map_or(0.0, |seg| seg.position_at(0));
        let (first, switches) = assert_no_alloc(|| {
            let first = keys.scale_at(START, position);
            (first, keys.switches_in(START, first, &segments))
        });
        switched += switches.len();
        let view = DeviceTransport {
            playing: true,
            scale: first,
            scale_switches: switches,
            ..DeviceTransport::default()
        };
        for seg in &segments {
            for tick in seg.from.0..seg.to.0 {
                let frame = seg.offset_of(Tick(tick));
                assert_eq!(
                    view.scale_at(frame),
                    keys.scale_at(START, tick as f64),
                    "tick {tick} at frame {frame}, block {size} at {rate} Hz"
                );
            }
        }
        t.end_block();
    }
    switched
}

#[test]
fn every_event_sees_the_key_of_its_own_tick_whatever_the_block_size() {
    let keys = map(&[(3840, D_MAJOR), (7680, 0)]);
    for rate in [44_100, 48_000] {
        for size in [1, 7, 64, 100, 128, 480, 512, 1021, 2048] {
            // About 20 seconds, across both changes.
            let blocks = (rate as usize * 20 / size).min(20_000);
            check(rate, size, blocks, &keys, |_, _| {});
        }
    }
}

#[test]
fn a_loop_wrapping_into_another_key_switches_at_the_wrap() {
    // Bar 1 to bar 2, the change at bar 2's start: a loop of the last 40 ticks of bar 1 and the
    // first 40 of bar 2 wraps across the two keys again and again, and a block holds several.
    let keys = map(&[(3840, D_MAJOR)]);
    let switched = check(44_100, 480, 400, &keys, |s, _| {
        s.playback.cue = Some(Tick(3800));
        s.playback.loop_region = LoopRegion::new(Tick(3800), Tick(3880));
    });
    assert!(
        switched > 100,
        "the loop crossed the change {switched} times"
    );
    // The same loop in blocks of other sizes: every tick still sees its own key.
    for size in [64, 100, 127, 1021] {
        check(48_000, size, 600, &keys, |s, _| {
            s.playback.cue = Some(Tick(3800));
            s.playback.loop_region = LoopRegion::new(Tick(3800), Tick(3880));
        });
    }
}

#[test]
fn a_seek_across_a_change_follows_the_key_at_the_new_place() {
    let keys = map(&[(3840, D_MAJOR)]);
    check(44_100, 512, 200, &keys, |s, _| {
        s.playback.cue = Some(Tick(100_000));
    });
    check(44_100, 512, 200, &keys, |s, _| {
        s.playback.cue = Some(Tick(3839));
    });
}
