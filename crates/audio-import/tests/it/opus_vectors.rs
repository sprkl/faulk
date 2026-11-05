//! The Opus decoder against the official RFC 8251 test vectors, by the procedure of libopus's
//! `tests/run_vectors.sh`: each of the 12 vectors decoded at 48 kHz in mono and in stereo,
//! converted to 16 bits as libopus does, and compared with `testvectorNN.dec` and
//! `testvectorNNm.dec` by `opus_compare` (below); a run passes when either comparison does.
//! The decoder is built as the pipeline builds it (`opuscule`, float, `libm`).
//!
//! The vectors are 75 MB, so they are not in the repository; the test is ignored by default:
//!
//! ```sh
//! curl -LO https://opus-codec.org/static/testvectors/opus_testvectors-rfc8251.tar.gz
//! tar -xzf opus_testvectors-rfc8251.tar.gz
//! FAULK_OPUS_VECTORS=opus_newvectors cargo test -p audio-import --release --test it opus_vectors:: -- --ignored --nocapture
//! ```
//!
//! Run it after an upgrade of `opuscule` or `libm` (#609). On 2026-11-05 with opuscule 0.2.2:
//! 24 of 24.

use opuscule::{Channels, Decoder, SampleRate};

/// The packets of an `opus_demo` bitstream: a 4-byte big-endian length, the encoder's final
/// range (4 bytes), the packet.
fn packets(bytes: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut at = 0;
    while at + 8 <= bytes.len() {
        let len = u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
        at += 8;
        out.push(&bytes[at..at + len]);
        at += len;
    }
    out
}

/// Decodes a vector to 16-bit samples, as `opus_demo -d 48000 <channels>` does.
fn decode(bits: &[u8], channels: usize) -> Vec<f32> {
    let layout = if channels == 1 {
        Channels::Mono
    } else {
        Channels::Stereo
    };
    let mut decoder = Box::new(Decoder::new(SampleRate::Hz48000, layout));
    let mut out = Vec::new();
    let mut last = 960;
    for packet in packets(bits) {
        let capacity = if packet.is_empty() { last } else { 5_760 };
        let mut pcm = vec![0f32; capacity * channels];
        let frames = decoder
            .decode((!packet.is_empty()).then_some(packet), &mut pcm, false)
            .expect("a vector's packet decodes");
        last = frames;
        out.extend(
            pcm[..frames * channels]
                .iter()
                .map(|x| (x * 32_768.0).round_ties_even().clamp(-32_768.0, 32_767.0)),
        );
    }
    out
}

fn read_pcm16(bytes: &[u8]) -> Vec<f32> {
    bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| f32::from(i16::from_le_bytes(*b)))
        .collect()
}

const NBANDS: usize = 21;
const NFREQS: usize = 240;
const BANDS: [usize; NBANDS + 1] = [
    0, 2, 4, 6, 8, 10, 12, 14, 16, 20, 24, 28, 32, 40, 48, 56, 68, 80, 96, 120, 156, 200,
];
const WIN: usize = 480;
const STEP: usize = 120;

/// Per band energy and power spectrum of `x` (interleaved, `nch` channels) over `nframes`
/// windows, by a direct DFT under a Hann window.
fn band_energy(out: Option<&mut [f32]>, ps: &mut [f32], x: &[f32], nch: usize, nframes: usize) {
    // The C's `3.14159265F`, which rounds to this `f32`.
    let pi = std::f32::consts::PI;
    let window: Vec<f32> = (0..WIN)
        .map(|j| 0.5 - 0.5 * ((2.0 * pi / (WIN as f32 - 1.0)) * j as f32).cos())
        .collect();
    let c: Vec<f32> = (0..WIN)
        .map(|j| ((2.0 * pi / WIN as f32) * j as f32).cos())
        .collect();
    let s: Vec<f32> = (0..WIN)
        .map(|j| ((2.0 * pi / WIN as f32) * j as f32).sin())
        .collect();
    let ps_sz = WIN / 2;
    let mut out = out;
    let mut buf = vec![0f32; nch * WIN];
    for xi in 0..nframes {
        for ci in 0..nch {
            for k in 0..WIN {
                buf[ci * WIN + k] = window[k] * x[(xi * STEP + k) * nch + ci];
            }
        }
        let mut xj = 0;
        for bi in 0..NBANDS {
            let mut p = [0f32; 2];
            while xj < BANDS[bi + 1] {
                for ci in 0..nch {
                    let (mut re, mut im) = (0f32, 0f32);
                    let mut ti = 0;
                    for k in 0..WIN {
                        re += c[ti] * buf[ci * WIN + k];
                        im -= s[ti] * buf[ci * WIN + k];
                        ti += xj;
                        if ti >= WIN {
                            ti -= WIN;
                        }
                    }
                    let v = re * re + im * im + 100_000.0;
                    ps[(xi * ps_sz + xj) * nch + ci] = v;
                    p[ci] += v;
                }
                xj += 1;
            }
            if let Some(out) = out.as_deref_mut() {
                for ci in 0..nch {
                    out[(xi * NBANDS + bi) * nch + ci] = p[ci] / (BANDS[bi + 1] - BANDS[bi]) as f32;
                }
            }
        }
    }
}

/// `opus_compare` at 48 kHz (libopus `src/opus_compare.c`, Copyright (c) 2011-2012 Xiph.Org
/// Foundation, Mozilla Corporation, BSD licence, translated): the quality of `decoded` against
/// the stereo `reference` (down-mixed for a mono decode), `None` when the vector fails.
fn opus_compare(reference: &[f32], decoded: &[f32], nch: usize) -> Option<f32> {
    let mut x = reference.to_vec();
    if nch == 1 {
        x = x
            .as_chunks::<2>()
            .0
            .iter()
            .map(|[l, r]| 0.5 * (l + r))
            .collect();
    }
    let xlength = x.len() / nch;
    assert_eq!(xlength, decoded.len() / nch, "sample counts do not match");
    let nframes = (xlength - WIN + STEP) / STEP;
    let mut xb = vec![0f32; nframes * NBANDS * nch];
    let mut big_x = vec![0f32; nframes * NFREQS * nch];
    let mut big_y = vec![0f32; nframes * NFREQS * nch];
    band_energy(Some(&mut xb), &mut big_x, &x, nch, nframes);
    band_energy(None, &mut big_y, decoded, nch, nframes);
    for xi in 0..nframes {
        let at = |bi: usize, ci: usize| (xi * NBANDS + bi) * nch + ci;
        for bi in 1..NBANDS {
            for ci in 0..nch {
                xb[at(bi, ci)] += 0.1 * xb[at(bi - 1, ci)];
            }
        }
        for bi in (0..NBANDS - 1).rev() {
            for ci in 0..nch {
                xb[at(bi, ci)] += 0.03 * xb[at(bi + 1, ci)];
            }
        }
        if xi > 0 {
            for bi in 0..NBANDS {
                for ci in 0..nch {
                    xb[at(bi, ci)] += 0.5 * xb[((xi - 1) * NBANDS + bi) * nch + ci];
                }
            }
        }
        if nch == 2 {
            for bi in 0..NBANDS {
                let (l, r) = (xb[at(bi, 0)], xb[at(bi, 1)]);
                xb[at(bi, 0)] += 0.01 * r;
                xb[at(bi, 1)] += 0.01 * l;
            }
        }
        for bi in 0..NBANDS {
            for xj in BANDS[bi]..BANDS[bi + 1] {
                for ci in 0..nch {
                    big_x[(xi * NFREQS + xj) * nch + ci] += 0.1 * xb[at(bi, ci)];
                    big_y[(xi * NFREQS + xj) * nch + ci] += 0.1 * xb[at(bi, ci)];
                }
            }
        }
    }
    for xj in 0..BANDS[NBANDS] {
        for ci in 0..nch {
            let (mut xt, mut yt) = (big_x[xj * nch + ci], big_y[xj * nch + ci]);
            for xi in 1..nframes {
                let i = (xi * NFREQS + xj) * nch + ci;
                let (x2, y2) = (big_x[i], big_y[i]);
                big_x[i] += xt;
                big_y[i] += yt;
                xt = x2;
                yt = y2;
            }
        }
    }
    let mut err = 0f64;
    for xi in 0..nframes {
        let mut ef = 0f64;
        for bi in 0..NBANDS {
            let mut eb = 0f64;
            for xj in BANDS[bi]..BANDS[bi + 1] {
                for ci in 0..nch {
                    let i = (xi * NFREQS + xj) * nch + ci;
                    let re = big_y[i] / big_x[i];
                    let mut im = (f64::from(re) - f64::from(re).ln() - 1.0) as f32;
                    if (79..=81).contains(&xj) {
                        im *= 0.1;
                    }
                    if xj == 80 {
                        im *= 0.1;
                    }
                    eb += f64::from(im);
                }
            }
            eb /= ((BANDS[bi + 1] - BANDS[bi]) * nch) as f64;
            ef += eb * eb;
        }
        ef /= NBANDS as f64;
        ef *= ef;
        err += ef * ef;
    }
    let err = (err / nframes as f64).powf(1.0 / 16.0);
    let q = (100.0 * (1.0 - 0.5 * (1.0 + err).ln() / 1.13f64.ln())) as f32;
    (q >= 0.0).then_some(q)
}

#[test]
#[ignore = "needs the RFC 8251 vectors (FAULK_OPUS_VECTORS): see the module's documentation"]
fn the_decoder_passes_the_rfc_8251_vectors() {
    let dir = std::path::PathBuf::from(
        std::env::var("FAULK_OPUS_VECTORS")
            .expect("FAULK_OPUS_VECTORS names the folder of the RFC 8251 vectors"),
    );
    let read =
        |name: String| std::fs::read(dir.join(&name)).unwrap_or_else(|e| panic!("{name}: {e}"));
    let mut passed = 0;
    for n in 1..=12 {
        let bits = read(format!("testvector{n:02}.bit"));
        let refs = [
            read_pcm16(&read(format!("testvector{n:02}.dec"))),
            read_pcm16(&read(format!("testvector{n:02}m.dec"))),
        ];
        for channels in [1, 2] {
            let decoded = decode(&bits, channels);
            let quality = refs
                .iter()
                .filter_map(|r| opus_compare(r, &decoded, channels))
                .reduce(f32::max);
            println!("testvector{n:02}, {channels} channel(s): {quality:?}");
            if quality.is_some() {
                passed += 1;
            }
        }
    }
    assert_eq!(passed, 24, "vectors passed");
}
