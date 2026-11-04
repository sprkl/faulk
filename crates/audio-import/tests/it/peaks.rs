//! The waveform peaks (AU-2, #111): their values on known signals, the coarser levels, the
//! file's header, size and validity, and that a stream gives the same peaks however it is cut.

use crate::support::*;
use audio_import::{MediaInfo, cache, peaks};

fn info(rate: u32, channels: u16, frames: u64) -> MediaInfo {
    MediaInfo {
        sample_rate: rate,
        channels,
        frames,
    }
}

/// The `(low, high)` of every peak of `level` for `channel`.
fn level(file: &[u8], info: &MediaInfo, level: usize, channel: usize) -> Vec<(i8, i8)> {
    let (at, len) = peaks::level_span(info, level);
    let stride = 2 * usize::from(info.channels);
    file[at..at + len]
        .chunks_exact(stride)
        .map(|p| (p[2 * channel] as i8, p[2 * channel + 1] as i8))
        .collect()
}

/// A signal whose peaks are known by hand: five whole blocks and a partial one, two channels.
#[test]
fn the_peaks_of_a_known_signal_are_its_lowest_and_highest_samples_rounded_outward() {
    let frames = 5 * 256 + 100;
    let mut samples = Vec::with_capacity(frames * 2);
    for i in 0..frames {
        let block = i / 256;
        let left = match block {
            // Silence.
            0 => 0.0,
            // A full-scale square.
            1 => {
                if i % 2 == 0 {
                    1.0
                } else {
                    -1.0
                }
            }
            // A constant 0.5: 63.5, so 63 below and 64 above.
            2 => 0.5,
            // Beyond full scale: clamped.
            3 => {
                if i == 3 * 256 + 7 {
                    -3.0
                } else {
                    2.0
                }
            }
            // A NaN is left out: 0.25 is 31.75.
            4 => {
                if i == 4 * 256 + 9 {
                    f32::NAN
                } else {
                    0.25
                }
            }
            // The partial block: silence.
            _ => 0.0,
        };
        samples.push(left);
        // The right channel is its own: a constant -0.5 (-63.5).
        samples.push(-0.5);
    }
    let info = info(48_000, 2, frames as u64);
    let file = peaks::of(&samples, &info).unwrap();
    assert_eq!(
        level(&file, &info, 0, 0),
        [(0, 0), (-127, 127), (63, 64), (-127, 127), (31, 32), (0, 0)]
    );
    assert_eq!(level(&file, &info, 0, 1), [(-64, -63); 6]);
    // Level 1: blocks 0 to 3, then 4 and the partial one.
    assert_eq!(level(&file, &info, 1, 0), [(-127, 127), (0, 32)]);
    assert_eq!(level(&file, &info, 1, 1), [(-64, -63); 2]);
    // Levels 2 and 3: one peak each, the whole signal.
    for l in [2, 3] {
        assert_eq!(level(&file, &info, l, 0), [(-127, 127)], "level {l}");
        assert_eq!(level(&file, &info, l, 1), [(-64, -63)], "level {l}");
    }
}

/// A 1 kHz sine of amplitude 0.8 at 48 kHz: 48 samples a period, so every block of 256
/// frames holds a crest (sample 12 of a period) and a trough (sample 36), 0.8 exactly:
/// 101.6, so -102 and 102 at every level.
#[test]
fn a_sine_gives_its_amplitude_at_every_level() {
    let frames = 48_000u64;
    let samples: Vec<f32> = (0..frames)
        .map(|i| 0.8 * (2.0 * std::f64::consts::PI * (i % 48) as f64 / 48.0).sin() as f32)
        .collect();
    let info = info(48_000, 1, frames);
    let file = peaks::of(&samples, &info).unwrap();
    for l in 0..peaks::LEVELS {
        let found = level(&file, &info, l, 0);
        assert_eq!(found.len() as u64, peaks::count(frames, l), "level {l}");
        // The last peak of a level may hold a partial block, still with a crest and a trough.
        assert!(
            found.iter().all(|p| *p == (-102, 102)),
            "level {l}: {found:?}"
        );
    }
}

/// The peaks of a coarser level computed directly from the samples of its blocks.
fn direct(samples: &[f32], channels: usize, block: usize) -> Vec<(i8, i8)> {
    samples
        .chunks(block * channels)
        .flat_map(|b| {
            (0..channels).map(move |c| {
                let ch = b.iter().skip(c).step_by(channels);
                let lo = ch.clone().fold(f32::INFINITY, |a, s| a.min(*s));
                let hi = ch.fold(f32::NEG_INFINITY, |a, s| a.max(*s));
                (peaks::low(lo), peaks::high(hi))
            })
        })
        .collect()
}

fn noise(n: usize, seed: u64) -> Vec<f32> {
    let mut x = seed;
    (0..n)
        .map(|_| {
            x = x.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            // Mostly quiet, now and then loud, so the levels differ.
            let v = ((x >> 40) as f32 / (1u64 << 24) as f32) * 2.0 - 1.0;
            if (x >> 20).is_multiple_of(97) {
                v
            } else {
                v * 0.1
            }
        })
        .collect()
}

#[test]
fn a_coarser_level_is_what_its_whole_blocks_would_give() {
    let channels = 3usize;
    let frames = 16_384 * 2 + 4_097;
    let samples = noise(frames * channels, 7);
    let info = info(44_100, channels as u16, frames as u64);
    let file = peaks::of(&samples, &info).unwrap();
    for l in 0..peaks::LEVELS {
        let block = peaks::block_frames(l) as usize;
        let expected = direct(&samples, channels, block);
        let found: Vec<(i8, i8)> = (0..expected.len() / channels)
            .flat_map(|i| {
                (0..channels).map({
                    let file = &file;
                    let info = &info;
                    move |c| level(file, info, l, c)[i]
                })
            })
            .collect();
        assert_eq!(found, expected, "level {l}");
    }
}

#[test]
fn a_stream_cut_anywhere_gives_the_same_peaks() {
    let frames = 10_000usize;
    let samples = noise(frames * 2, 3);
    let info = info(48_000, 2, frames as u64);
    let whole = peaks::of(&samples, &info).unwrap();
    for cut in [1usize, 100, 255, 256, 257, 4096] {
        let mut builder = peaks::PeakBuilder::new(2);
        for part in samples.chunks(cut * 2) {
            builder.push(part);
        }
        assert_eq!(builder.frames(), frames as u64);
        assert_eq!(
            builder.finish(&info).as_deref(),
            Some(&whole[..]),
            "cut {cut}"
        );
    }
}

#[test]
fn the_header_names_the_media_the_decoder_and_the_levels() {
    let info = info(44_100, 2, 22_050);
    let h = peaks::header(&info);
    assert_eq!(&h[..8], b"FKPEAK01");
    assert_eq!(u32::from_le_bytes(h[8..12].try_into().unwrap()), 44_100);
    assert_eq!(u16::from_le_bytes(h[12..14].try_into().unwrap()), 2);
    assert_eq!(u16::from_le_bytes(h[14..16].try_into().unwrap()), 4);
    assert_eq!(u64::from_le_bytes(h[16..24].try_into().unwrap()), 22_050);
    assert_eq!(
        u32::from_le_bytes(h[24..28].try_into().unwrap()),
        cache::DECODER
    );
    assert_eq!(u16::from_le_bytes(h[28..30].try_into().unwrap()), 256);
    assert_eq!(u16::from_le_bytes(h[30..32].try_into().unwrap()), 4);
    // 87, 22, 6 and 2 peaks of 4 bytes.
    assert_eq!(peaks::size(&info), Some(32 + (87 + 22 + 6 + 2) * 4));
    assert_eq!(
        peaks::path(&project::Hash::parse(&"ab".repeat(32)).unwrap()),
        format!(".faulk/cache/peaks/{}.peaks", "ab".repeat(32))
    );
}

#[test]
fn only_a_whole_file_of_the_media_and_of_this_decoder_is_valid() {
    let (imported, _) = import(read("sine.mp3"), Some("mp3")).unwrap();
    let info = imported.info;
    let file = imported.peaks.unwrap();
    assert_eq!(Some(file.len() as u64), peaks::size(&info));
    assert!(peaks::is_valid(&file, file.len() as u64, &info));
    // Cut short, or longer.
    assert!(!peaks::is_valid(&file, file.len() as u64 - 1, &info));
    assert!(!peaks::is_valid(&file, file.len() as u64 + 1, &info));
    // A header shorter than its size.
    assert!(!peaks::is_valid(&file[..31], file.len() as u64, &info));
    // Another media, or another decoder: each byte of the header counts.
    for byte in 0..peaks::HEADER_BYTES {
        let mut other = file.clone();
        other[byte] ^= 1;
        assert!(
            !peaks::is_valid(&other, other.len() as u64, &info),
            "byte {byte}"
        );
    }
    let longer = MediaInfo {
        frames: info.frames + 1,
        ..info
    };
    assert!(!peaks::is_valid(&file, file.len() as u64, &longer));
}

#[test]
fn what_is_not_the_media_gives_no_peaks() {
    let samples = noise(1000 * 2, 1);
    let mut builder = peaks::PeakBuilder::new(2);
    builder.push(&samples);
    assert!(builder.finish(&info(48_000, 2, 999)).is_none());
    assert!(peaks::of(&samples, &info(48_000, 1, 2000)).is_some());
    assert!(peaks::of(&samples, &info(48_000, 4, 500)).is_some());
    assert!(peaks::of(&samples, &info(48_000, 2, 1001)).is_none());
    assert!(
        peaks::PeakBuilder::new(0)
            .finish(&info(48_000, 0, 0))
            .is_none()
    );
}

/// The criterion: about 45 KB a minute at the finest level. Measured on a minute of stereo.
#[test]
fn a_minute_of_stereo_is_about_45_kb_at_the_finest_level() {
    for (rate, expected) in [(48_000u32, 45_000usize), (44_100, 41_344)] {
        let frames = u64::from(rate) * 60;
        let samples = noise(frames as usize * 2, 11);
        let info = info(rate, 2, frames);
        let file = peaks::of(&samples, &info).unwrap();
        let (_, finest) = peaks::level_span(&info, 0);
        println!(
            "one minute of stereo at {rate} Hz: {finest} bytes at level 0, {} bytes in all",
            file.len()
        );
        assert_eq!(finest, expected);
        assert_eq!(file.len() as u64, peaks::size(&info).unwrap());
    }
}

/// Every fixture's import gives the peaks of its decoded samples; for a lossless file, they
/// are the peaks of the known source signal itself (a 440/660 Hz sine of amplitude 0.5, so -64
/// and 64 in every whole block). A lossy file's are its decode's (the codec's error, largest
/// at the start of an AAC file, moves them away from the source's).
#[test]
fn the_import_gives_the_peaks_of_what_it_decoded() {
    for (name, lossless) in [
        ("sine-s16.wav", true),
        ("sine-s24.aiff", true),
        ("sine.flac", true),
        ("sine-48k-mono.flac", true),
        ("sine.mp3", false),
        ("sine.m4a", false),
        ("sine.ogg", false),
    ] {
        let (imported, pcm) = import(read(name), extension(name)).unwrap();
        let info = imported.info;
        assert_eq!(imported.peaks, peaks::of(&pcm, &info), "{name}");
        if !lossless {
            continue;
        }
        let file = imported.peaks.unwrap();
        let signal = source(
            info.sample_rate,
            info.frames as usize,
            usize::from(info.channels),
        );
        assert_eq!(Some(&file), peaks::of(&signal, &info).as_ref(), "{name}");
        let mut whole = level(&file, &info, 0, 0);
        // The last block (34 frames of 22 050) may hold no crest.
        whole.pop();
        assert!(whole.iter().all(|p| *p == (-64, 64)), "{name}: {whole:?}");
    }
}

/// The envelope holds every sample: each sample lies between the low and the high of its
/// block (scaled by 127), at every level, whatever the input, infinities and NaN included.
#[test]
fn every_sample_lies_inside_its_peak() {
    let channels = 2usize;
    let frames = 20_000usize;
    let mut samples = noise(frames * channels, 5);
    // Out-of-range values and infinities are clamped; NaN is left out.
    samples[10] = f32::INFINITY;
    samples[999] = f32::NEG_INFINITY;
    samples[4_000] = 7.5;
    samples[4_001] = f32::NAN;
    let info = info(48_000, channels as u16, frames as u64);
    let file = peaks::of(&samples, &info).unwrap();
    for l in 0..peaks::LEVELS {
        let block = peaks::block_frames(l) as usize;
        for c in 0..channels {
            let found = level(&file, &info, l, c);
            for (i, s) in samples.iter().skip(c).step_by(channels).enumerate() {
                if s.is_nan() {
                    continue;
                }
                let (lo, hi) = found[i / block];
                let v = (s * peaks::SCALE).clamp(-peaks::SCALE, peaks::SCALE);
                assert!(
                    f32::from(lo) <= v && v <= f32::from(hi),
                    "level {l}, channel {c}, frame {i}: {v} outside {lo}..{hi}"
                );
            }
        }
    }
}

/// A block of NaN only shows nothing (0, 0), not a full-scale spike from the sentinels.
#[test]
fn a_block_of_nan_only_is_silence() {
    let mut samples = vec![f32::NAN; 256];
    samples.extend(std::iter::repeat_n(0.5, 256));
    let info = info(48_000, 1, 512);
    let file = peaks::of(&samples, &info).unwrap();
    assert_eq!(level(&file, &info, 0, 0), [(0, 0), (63, 64)]);
    assert_eq!(level(&file, &info, 1, 0), [(0, 64)]);
}

/// The most channels a project takes, and a slice that ends inside a frame.
#[test]
fn sixty_four_channels_and_a_partial_frame() {
    let channels = project::limits::MAX_CHANNELS;
    let frames = 300usize;
    let samples: Vec<f32> = (0..frames * usize::from(channels))
        .map(|i| (i % usize::from(channels)) as f32 / 64.0)
        .collect();
    let info = info(48_000, channels, frames as u64);
    let mut builder = peaks::PeakBuilder::new(channels);
    // A slice ending inside a frame: the partial frame is left out of that push.
    builder.push(&samples[..usize::from(channels) * 10 + 3]);
    assert_eq!(builder.frames(), 10);
    builder.push(&samples[usize::from(channels) * 10..]);
    let file = builder.finish(&info).unwrap();
    assert_eq!(Some(file.len() as u64), peaks::size(&info));
    for c in 0..usize::from(channels) {
        let v = c as f32 / 64.0;
        assert_eq!(
            level(&file, &info, 0, c),
            [(peaks::low(v), peaks::high(v)); 2],
            "channel {c}"
        );
    }
}
