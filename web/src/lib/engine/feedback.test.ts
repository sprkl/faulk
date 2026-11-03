import { describe, expect, it } from 'vitest'
import {
  ACTIVE_CLIP_BYTES,
  FEEDBACK_HEADER_BYTES,
  FEEDBACK_VERSION,
  MAX_RECORDED_PER_REPORT,
  RECORDED_NOTE_BYTES,
  TRACK_METER_BYTES,
  decodeFeedback,
  peakOf,
  readFeedback,
} from './feedback'

/** An active clip as the engine writes it: track u32, scene u16, state u8, padding, anchor i64. */
type Cell = [track: number, scene: number, state: number, anchor?: number]

/** A report as the engine lays it out (crates/engine/src/msg.rs), for `tracks` track meters and the grid `cells`. */
function report(tracks: number, edit?: (view: DataView) => void, cells: Cell[] = []): ArrayBuffer {
  const clipsAt = FEEDBACK_HEADER_BYTES + (tracks + 1) * TRACK_METER_BYTES
  const buffer = new ArrayBuffer(clipsAt + cells.length * ACTIVE_CLIP_BYTES)
  const view = new DataView(buffer)
  view.setUint8(0, FEEDBACK_VERSION)
  view.setUint32(4, 8, true) // seq
  view.setUint32(40, tracks, true)
  view.setUint32(44, cells.length, true)
  cells.forEach(([track, scene, state, anchor = 0], i) => {
    const at = clipsAt + i * ACTIVE_CLIP_BYTES
    view.setUint32(at, track, true)
    view.setUint16(at + 4, scene, true)
    view.setUint8(at + 6, state)
    view.setBigInt64(at + 8, BigInt(anchor), true)
  })
  for (let i = 0; i <= tracks; i++) {
    const at = FEEDBACK_HEADER_BYTES + i * TRACK_METER_BYTES
    view.setFloat32(at, 0.1 * (i + 1), true)
    view.setFloat32(at + 4, 0.05 * (i + 1), true)
    view.setFloat32(at + 8, 0.02 * (i + 1), true)
    view.setFloat32(at + 12, 0.01 * (i + 1), true)
    view.setFloat32(at + 16, 3 * (i + 1), true)
  }
  edit?.(view)
  return buffer
}

/** A note as the engine writes it after the clips: track, start, length, pitch, velocity. */
type Played = [track: number, start: number, length: number, pitch: number, velocity: number]

/** `report` with the take's notes after the cells, and its number, count and capture flag. */
function withNotes(tracks: number, cells: Cell[], notes: Played[], take: number, total: number, capturing: boolean): ArrayBuffer {
  const base = new Uint8Array(report(tracks, undefined, cells))
  const buffer = new ArrayBuffer(base.length + notes.length * RECORDED_NOTE_BYTES)
  new Uint8Array(buffer).set(base)
  const view = new DataView(buffer)
  view.setUint32(48, notes.length, true)
  view.setUint32(56, take, true)
  view.setUint32(60, total, true)
  view.setUint32(64, 2, true)
  view.setUint32(68, capturing ? 1 : 0, true)
  notes.forEach(([track, start, length, pitch, velocity], i) => {
    const at = base.length + i * RECORDED_NOTE_BYTES
    view.setUint32(at, track, true)
    view.setBigInt64(at + 8, BigInt(start), true)
    view.setBigInt64(at + 16, BigInt(length), true)
    view.setUint8(at + 24, pitch)
    view.setUint8(at + 25, velocity)
  })
  return buffer
}

describe('decodeFeedback', () => {
  it('reads the take: its number, its count, the lost notes, the capture and its last notes numbered from the count', () => {
    const r = decodeFeedback(
      withNotes(
        2,
        [[0, 1, 2, 3840]],
        [
          [1, -20, 480, 60, 100],
          [0, 7680, 1, 127, 1],
        ],
        3,
        9,
        true,
      ),
    )!
    expect(r).toMatchObject({ recordTake: 3, recordTotal: 9, recordDropped: 2, recordCapturing: true })
    expect(r.clips).toEqual([{ track: 0, scene: 1, state: 'playing', anchor: 3840 }])
    expect(r.recorded).toEqual([
      { index: 7, track: 1, start: -20, length: 480, pitch: 60, velocity: 100 },
      { index: 8, track: 0, start: 7680, length: 1, pitch: 127, velocity: 1 },
    ])
    // A count beyond the bytes, or beyond what a report carries, is refused.
    const short = withNotes(0, [], [[0, 0, 1, 60, 90]], 1, 1, false)
    new DataView(short).setUint32(48, 2, true)
    expect(decodeFeedback(short)).toBeNull()
    const many = withNotes(0, [], [], 1, 1, false)
    new DataView(many).setUint32(48, MAX_RECORDED_PER_REPORT + 1, true)
    expect(decodeFeedback(many)).toBeNull()
    expect(decodeFeedback(report(0))!.recorded).toEqual([])
  })

  it('reads the header, every track meter and the output meter after them', () => {
    const r = decodeFeedback(
      report(3, (v) => {
        v.setUint8(1, 1)
        v.setUint16(2, 7, true)
        v.setBigInt64(8, 1920n, true)
        v.setBigUint64(16, 96000n, true)
        v.setFloat32(24, 0.25, true)
        v.setUint32(28, 2, true)
        v.setUint32(32, 3, true)
        v.setUint32(36, 4, true)
        v.setUint32(52, 5, true)
      }),
    )!
    expect(r).toMatchObject({
      seq: 8,
      epoch: 7,
      transport: 'playing',
      playheadTick: 1920,
      playheadFrame: 96000,
      cpuLoad: 0.25,
      liveOverflows: 2,
      rejected: 3,
      diskUnderruns: 4,
      voices: 5,
    })
    expect(r.tracks).toHaveLength(3)
    expect(r.tracks[2].peakL).toBeCloseTo(0.3)
    expect(r.tracks[1].rmsR).toBeCloseTo(0.02)
    expect(r.master.peakL).toBeCloseTo(0.4)
    expect(peakOf(r.master)).toBeCloseTo(0.4)
    expect(peakOf(r.tracks[0])).toBeCloseTo(0.1)
    // The gain reduction is the fifth float of each meter, the output's included.
    expect(r.tracks.map((m) => m.gainReductionDb)).toEqual([3, 6, 9])
    expect(r.master.gainReductionDb).toBe(12)
  })

  it('reads a corrupt reduction (negative, not a number) as none', () => {
    const at = FEEDBACK_HEADER_BYTES
    expect(decodeFeedback(report(1, (v) => v.setFloat32(at + 16, -4, true)))!.tracks[0].gainReductionDb).toBe(0)
    expect(decodeFeedback(report(1, (v) => v.setFloat32(at + 16, NaN, true)))!.tracks[0].gainReductionDb).toBe(0)
  })

  it('refuses a report of the previous layouts (version 4: a 56-byte header; version 3)', () => {
    expect(decodeFeedback(report(2, (v) => v.setUint8(0, 4)))).toBeNull()
    expect(decodeFeedback(report(2, (v) => v.setUint8(0, 3)))).toBeNull()
  })

  it('reads a negative playhead (a count-in) and an unknown transport as stopped', () => {
    const r = decodeFeedback(
      report(0, (v) => {
        v.setBigInt64(8, -3840n, true)
        v.setUint8(1, 9)
      }),
    )!
    expect(r.playheadTick).toBe(-3840)
    expect(r.transport).toBe('stopped')
    expect(r.tracks).toEqual([])
  })

  it('says not measured for a negative load and clamps one above 1', () => {
    expect(decodeFeedback(report(0, (v) => v.setFloat32(24, -1, true)))!.cpuLoad).toBeNull()
    expect(decodeFeedback(report(0, (v) => v.setFloat32(24, 3, true)))!.cpuLoad).toBe(1)
    expect(decodeFeedback(report(0, (v) => v.setFloat32(24, 0, true)))!.cpuLoad).toBe(0)
  })

  it('refuses another version, a short buffer and a track count beyond the bytes', () => {
    expect(decodeFeedback(report(1, (v) => v.setUint8(0, FEEDBACK_VERSION - 1)))).toBeNull()
    expect(decodeFeedback(report(1, (v) => v.setUint8(0, FEEDBACK_VERSION + 1)))).toBeNull()
    expect(decodeFeedback(report(2).slice(0, FEEDBACK_HEADER_BYTES + 2 * TRACK_METER_BYTES))).toBeNull()
    expect(decodeFeedback(report(1, (v) => v.setUint32(40, 2, true)))).toBeNull()
    expect(decodeFeedback(report(1, (v) => v.setUint32(40, 0xffffffff, true)))).toBeNull()
    expect(decodeFeedback(new ArrayBuffer(0))).toBeNull()
    expect(decodeFeedback(new ArrayBuffer(FEEDBACK_HEADER_BYTES))).toBeNull()
  })

  it('decodes at an offset of a larger buffer (the shared memory)', () => {
    const body = new Uint8Array(report(1))
    const memory = new Uint8Array(64 + body.length + 32)
    memory.set(body, 64)
    expect(decodeFeedback(memory.buffer, 64, body.length)!.tracks).toHaveLength(1)
  })
})

describe('decodeFeedback, the launch grid', () => {
  it('reads every cell that is not idle, with its state', () => {
    const r = decodeFeedback(
      report(2, undefined, [
        [0, 3, 2, 3840],
        [0, 4, 1, 7680],
        [1, 300, 3, -960],
        [1, 0, 4],
      ]),
    )!
    expect(r.clips).toEqual([
      { track: 0, scene: 3, state: 'playing', anchor: 3840 },
      { track: 0, scene: 4, state: 'queued', anchor: 7680 },
      { track: 1, scene: 300, state: 'stopping', anchor: -960 },
      { track: 1, scene: 0, state: 'armed', anchor: 0 },
    ])
  })

  it('reads a track the grid holds with no clip as held, in no cell', () => {
    const r = decodeFeedback(
      report(2, undefined, [
        [1, 65535, 5],
        [0, 3, 2, 3840],
      ]),
    )!
    expect(r.held).toEqual([1])
    expect(r.clips).toEqual([{ track: 0, scene: 3, state: 'playing', anchor: 3840 }])
  })

  it('reads a report without cells (an older engine writes none) as an idle grid', () => {
    expect(decodeFeedback(report(2))!.clips).toEqual([])
  })

  it('refuses a state it does not know, more cells than two per track, and cells cut short', () => {
    for (const state of [0, 6, 255]) expect(decodeFeedback(report(1, undefined, [[0, 0, state]]))).toBeNull()
    const three: Cell[] = [
      [0, 0, 2],
      [0, 1, 1],
      [0, 2, 1],
    ]
    expect(decodeFeedback(report(1, undefined, three))).toBeNull()
    expect(decodeFeedback(report(1, undefined, [[0, 0, 2]]).slice(0, -1))).toBeNull()
    // A count that announces more cells than the bytes hold.
    expect(decodeFeedback(report(1, (v) => v.setUint32(44, 2, true), [[0, 0, 2]]))).toBeNull()
  })
})

describe('readFeedback', () => {
  const shared = (r: ArrayBuffer, seq: number) => {
    const memory = new SharedArrayBuffer(r.byteLength + 8)
    new Uint8Array(memory, 8).set(new Uint8Array(r))
    new DataView(memory, 8).setUint32(4, seq, true)
    return memory
  }

  it('reads a whole report', () => {
    expect(readFeedback(shared(report(2), 10), 8, report(2).byteLength)!.seq).toBe(10)
  })

  it('reads the cells of the grid too, from a region larger than the report', () => {
    const r = report(2, undefined, [[1, 2, 2]])
    // The region is sized for the most cells (two per track): the report is copied up to its own.
    const region = new SharedArrayBuffer(r.byteLength + 8 + 4 * ACTIVE_CLIP_BYTES)
    new Uint8Array(region, 8).set(new Uint8Array(r))
    new DataView(region, 8).setUint32(4, 10, true)
    expect(readFeedback(region, 8, region.byteLength - 8)!.clips).toEqual([{ track: 1, scene: 2, state: 'playing', anchor: 0 }])
    // A count above the room (two per track) is refused, never read past the region.
    new DataView(region, 8).setUint32(44, 4_000_000_000, true)
    expect(readFeedback(region, 8, region.byteLength - 8)).toBeNull()
  })

  it('reads nothing while a write is in progress (odd seq), before the first (0) or past the region', () => {
    const len = report(2).byteLength
    expect(readFeedback(shared(report(2), 11), 8, len)).toBeNull()
    expect(readFeedback(shared(report(2), 0), 8, len)).toBeNull()
    // The tracks word announces more than the region holds: nothing, never a read past it.
    const big = report(2, (v) => v.setUint32(40, 5000, true))
    expect(readFeedback(shared(big, 10), 8, big.byteLength)).toBeNull()
  })
})
