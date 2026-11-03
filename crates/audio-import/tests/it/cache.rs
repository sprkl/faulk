//! The PCM cache's header and its validity: a cache is used only when it is whole and of the
//! same media and decoder; anything else is made again.

use crate::support::*;
use audio_import::{MediaInfo, cache};

fn info() -> MediaInfo {
    MediaInfo {
        sample_rate: 44_100,
        channels: 2,
        frames: 22_050,
    }
}

fn whole(info: &MediaInfo, pcm: &[f32]) -> Vec<u8> {
    let mut file = cache::header(info).to_vec();
    cache::encode(pcm, &mut file);
    file
}

#[test]
fn a_whole_cache_of_the_media_is_valid_and_holds_the_decoded_samples() {
    let (imported, pcm) = import(read("sine.mp3"), Some("mp3")).unwrap();
    let file = whole(&imported.info, &pcm);
    assert_eq!(Some(file.len() as u64), cache::size(&imported.info));
    assert!(cache::is_valid(
        &file[..cache::HEADER_BYTES],
        file.len() as u64,
        &imported.info
    ));
    // The samples come back as they were decoded.
    let back: Vec<f32> = file[cache::HEADER_BYTES..]
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect();
    assert!(back == pcm);
    assert_eq!(
        cache::path(&imported.hash),
        format!(".faulk/cache/pcm/{}.pcm", imported.hash)
    );
}

#[test]
fn any_difference_makes_the_cache_invalid() {
    let info = info();
    let file = whole(&info, &vec![0.25; 22_050 * 2]);
    let len = file.len() as u64;
    let head = &file[..cache::HEADER_BYTES];
    assert!(cache::is_valid(head, len, &info));
    // Cut short, one byte too long, or a header cut.
    assert!(!cache::is_valid(head, len - 4, &info));
    assert!(!cache::is_valid(head, len + 1, &info));
    assert!(!cache::is_valid(&head[..31], len, &info));
    // Another media: rate, channels, frames.
    for other in [
        MediaInfo {
            sample_rate: 48_000,
            ..info
        },
        MediaInfo {
            channels: 1,
            ..info
        },
        MediaInfo {
            frames: 22_049,
            ..info
        },
    ] {
        assert!(!cache::is_valid(head, len, &other), "{other:?}");
    }
    // Every byte of the header counts: magic, fields, the decoder's version, the reserved bytes.
    for at in 0..cache::HEADER_BYTES {
        let mut changed = head.to_vec();
        changed[at] ^= 0x01;
        assert!(!cache::is_valid(&changed, len, &info), "byte {at}");
    }
    // A cache of another decoder's version is made again.
    let mut older = head.to_vec();
    older[24..28].copy_from_slice(&(cache::DECODER - 1).to_le_bytes());
    assert!(!cache::is_valid(&older, len, &info));
}

#[test]
fn a_size_that_would_overflow_has_none() {
    let huge = MediaInfo {
        sample_rate: 44_100,
        channels: 64,
        frames: u64::MAX / 8,
    };
    assert_eq!(cache::size(&huge), None);
    assert!(!cache::is_valid(&cache::header(&huge), u64::MAX, &huge));
}
