"""Writes the reference files of the import tests (AU-1, #110). Not run by the tests or CI: the
files it wrote are committed. Rerun only to change them, then update `expected.json` with
`FAULK_BLESS=1 cargo test -p audio-import --test it fixtures::`.

Needs Python 3 with `numpy`, `soundfile` (libsndfile: WAV, AIFF, FLAC, Ogg Vorbis) and `av`
(PyAV, FFmpeg's encoders: MP3 with LAME, AAC, ALAC, Opus):

    python -m venv env && env/bin/pip install numpy soundfile av
    env/bin/python make_fixtures.py

The source signal is the same for every file: half a second at 44.1 kHz, stereo, a 440 Hz sine
on the left and a 660 Hz sine on the right at half of full scale, as 16-bit integers (`source`
below; `sine-s16.wav` holds it exactly). The lossless files hold exactly these samples; the
lossy ones are compared with them at a tolerance (tests/it/fixtures.rs).

`recorded.webm` is not written here: it is what a browser records (#609), made once in Chromium 156
(Playwright's) by MediaRecorder from two oscillators, 440 Hz left and 660 Hz right at half scale:

    const ctx = new AudioContext({ sampleRate: 48000 })
    const dest = ctx.createMediaStreamDestination()
    const merger = ctx.createChannelMerger(2)
    for (const [hz, ch] of [[440, 0], [660, 1]]) {
      const o = ctx.createOscillator(); o.frequency.value = hz
      const g = ctx.createGain(); g.gain.value = 0.5
      o.connect(g).connect(merger, 0, ch); o.start()
    }
    merger.connect(dest)
    const rec = new MediaRecorder(dest.stream, { mimeType: 'audio/webm;codecs=opus' })
    // start, stop after one second, keep the blob's bytes
"""

import os

import av
import numpy as np
import soundfile as sf

HERE = os.path.dirname(os.path.abspath(__file__))
RATE = 44_100
FRAMES = RATE // 2


def source(rate=RATE, frames=FRAMES, channels=2):
    t = np.arange(frames) / rate
    left = 0.5 * np.sin(2 * np.pi * 440 * t)
    right = 0.5 * np.sin(2 * np.pi * 660 * t)
    both = np.stack([left, right], axis=1)[:, :channels]
    return np.round(both * 32767).astype(np.int16)


def out(name):
    return os.path.join(HERE, name)


def encode(name, codec, samples, rate, fmt, layout="stereo", bit_rate=None, container=None):
    with av.open(out(name), "w", format=container) as box:
        stream = box.add_stream(codec, rate=rate)
        stream.layout = layout
        if bit_rate:
            stream.bit_rate = bit_rate
        frame_size = stream.codec_context.frame_size or 4096
        data = samples.astype(np.float32) / 32768.0 if fmt == "fltp" else samples
        at = 0
        while at < len(data):
            chunk = data[at : at + frame_size]
            if fmt.endswith("p"):
                frame = av.AudioFrame.from_ndarray(np.ascontiguousarray(chunk.T), format=fmt, layout=layout)
            else:
                frame = av.AudioFrame.from_ndarray(np.ascontiguousarray(chunk.reshape(1, -1)), format=fmt, layout=layout)
            frame.sample_rate = rate
            frame.pts = at
            for packet in stream.encode(frame):
                box.mux(packet)
            at += len(chunk)
        for packet in stream.encode(None):
            box.mux(packet)


def main():
    s = source()
    sf.write(out("sine-s16.wav"), s, RATE, subtype="PCM_16")
    sf.write(out("sine-s24.wav"), s, RATE, subtype="PCM_24")
    sf.write(out("sine-f32.wav"), s.astype(np.float32) / 32768.0, RATE, subtype="FLOAT")
    sf.write(out("sine-u8.wav"), s, RATE, subtype="PCM_U8")
    sf.write(out("sine-s16.aiff"), s, RATE, subtype="PCM_16", format="AIFF")
    sf.write(out("sine-s24.aiff"), s, RATE, subtype="PCM_24", format="AIFF")
    sf.write(out("sine.flac"), s, RATE, subtype="PCM_16")
    sf.write(out("sine.ogg"), s.astype(np.float32) / 32768.0, RATE, format="OGG", subtype="VORBIS")
    mono48 = source(rate=48_000, frames=24_000, channels=1)
    sf.write(out("sine-48k-mono.flac"), mono48, 48_000, subtype="PCM_16")
    encode("sine.mp3", "libmp3lame", s, RATE, "fltp", bit_rate=128_000)
    encode("sine.m4a", "aac", s, RATE, "fltp", bit_rate=128_000)
    encode("sine-alac.m4a", "alac", s, RATE, "s16p")
    encode("sine.opus", "libopus", source(rate=48_000, frames=24_000), 48_000, "fltp", bit_rate=96_000, container="ogg")
    # Opus (#609): the same stream in WebM (FFmpeg's muxer writes CodecDelay and the last block's
    # DiscardPadding), and a mono stream in Ogg.
    encode("sine.webm", "libopus", source(rate=48_000, frames=24_000), 48_000, "fltp", bit_rate=96_000, container="webm")
    encode("sine-mono.opus", "libopus", source(rate=48_000, frames=24_000, channels=1), 48_000, "fltp", layout="mono", bit_rate=64_000, container="ogg")
    with open(out("not-audio.txt"), "w", newline="\n") as f:
        f.write("This is a text file, not audio.\n")


if __name__ == "__main__":
    main()
