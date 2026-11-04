// The export's render (issue #297) through the shipped engine module and the project core, both
// built by `pnpm test`, under Node: a song made by commands is exported as the Worker does it,
// and the file is read back.
import { readFileSync } from 'node:fs'
import { beforeAll, describe, expect, it } from 'vitest'
import { SINGLE_SYNTH, type Command } from '../generated/protocol'
import { coreModule, ProjectCore, type ExportRequest } from '../project/core'
import type { Dir } from '../audio/stream/executor'
import { renderExport, type FromExport, type ToExport } from './export-render'

const engine = new Uint8Array(readFileSync(new URL('../engine/wasm/engine.wasm', import.meta.url))).buffer
let module: WebAssembly.Module

beforeAll(async () => {
  module = await coreModule(readFileSync(new URL('../project/wasm/project.wasm', import.meta.url)))
})

const seed = { timeMs: 1_760_000_000_000, random: [1, 2, 3] as [number, number, number] }

/** A core with a two-bar clip of four notes on the synth. */
function song(): ProjectCore {
  const core = ProjectCore.create(module, 'single_synth', 1, seed)
  const run = (command: Command) => {
    const { report } = core.run(command)
    if (report.status !== 'applied') throw new Error(JSON.stringify(report))
    return report.output
  }
  const created = run({ type: 'clip.create', track: SINGLE_SYNTH.track, at: { at: 'arrangement', start: 0 }, length: 7680 })
  if (created.type !== 'clip.create') throw new Error('a clip')
  const note = (start: number, pitch: number) => ({ start, pitch, length: 900, velocity: 100, cents: 0, chance: 100, repeat: 1 })
  run({ type: 'clip.notes.add', clip: created.clip, notes: [note(0, 57), note(1920, 60), note(3840, 64), note(5760, 69)] })
  return core
}

/** Runs the render as the Worker does and returns what it posted. */
async function render(core: ProjectCore, request: ExportRequest, room?: () => Promise<void>, folder: Dir | null = null) {
  const job = core.exportJob(request)
  if (job.status !== 'ready') throw new Error(`not ready: ${JSON.stringify(job)}`)
  const messages: FromExport[] = []
  const start: ToExport = {
    type: 'start',
    module: engine.slice(0),
    sampleRate: job.plan.settings.sample_rate,
    bits: { pcm16: 16, pcm24: 24, float32: 32 }[job.plan.settings.bit_depth],
    bodyFrames: job.plan.body_frames,
    setup: job.setup.slice().buffer,
    start: job.start.slice().buffer,
    stop: job.stop.slice().buffer,
    folder: folder as unknown as FileSystemDirectoryHandle | null,
  }
  await renderExport(start, (message) => messages.push(message), room)
  return { job, messages }
}

function wav(messages: FromExport[]) {
  const done = messages.find((m) => m.type === 'done')
  if (done?.type !== 'done') throw new Error(`no done: ${JSON.stringify(messages.at(-1))}`)
  const chunks = messages.filter((m) => m.type === 'chunk').map((m) => new Uint8Array(m.bytes))
  const header = new Uint8Array(done.header)
  const file = new Uint8Array(header.length + chunks.reduce((n, c) => n + c.length, 0))
  file.set(header)
  let at = header.length
  for (const chunk of chunks) {
    file.set(chunk, at)
    at += chunk.length
  }
  return { file, done, view: new DataView(file.buffer) }
}

describe('the export render', () => {
  it('writes a WAV file of the song: its header, its length, its level', async () => {
    const { job, messages } = await render(song(), {})
    const { file, done, view } = wav(messages)
    expect(String.fromCharCode(...file.subarray(0, 4))).toBe('RIFF')
    expect(String.fromCharCode(...file.subarray(8, 16))).toBe('WAVEfmt ')
    expect(view.getUint32(4, true)).toBe(file.length - 8)
    expect(view.getUint16(20, true)).toBe(1)
    expect(view.getUint16(22, true)).toBe(2)
    expect(view.getUint32(24, true)).toBe(48_000)
    expect(view.getUint16(34, true)).toBe(24)
    expect(String.fromCharCode(...file.subarray(36, 40))).toBe('data')
    expect(view.getUint32(40, true)).toBe(file.length - 44)
    // Two bars at 120 BPM: four seconds, and the synth's release after the last note.
    expect(job.plan.body_frames).toBe(4 * 48_000)
    const frames = (file.length - 44) / 6
    expect(frames).toBe(done.written)
    expect(frames).toBeGreaterThanOrEqual(4 * 48_000)
    expect(frames).toBeLessThan(6 * 48_000)
    // Not silent: the first note sounds at the start and the last one in the last half second.
    const sample = (frame: number) => {
      const at = 44 + frame * 6
      return Math.abs((view.getInt32(at - 1, true) >> 8) / 8_388_608)
    }
    const peak = (from: number, to: number) => Math.max(...Array.from({ length: to - from }, (_, i) => sample(from + i)))
    expect(peak(0, 4800)).toBeGreaterThan(0.01)
    expect(peak(3.5 * 48_000, 4 * 48_000 - 1)).toBeGreaterThan(0.01)
    expect(done.peak).toBeGreaterThan(0.01)
    expect(done.peak).toBeLessThanOrEqual(1)
    expect(done.clipped).toBe(0)
    // A mix under the ceiling never engages the output limiter, so the gain reduction is 0 (#402).
    expect(done.reduction).toBe(0)
    // The shipped module's count of frames the output protection silenced for a non-finite sample: a number, 0 on a clean song.
    expect(done.silenced).toBe(0)
  })

  it('reports its progress up to the end of the song, then the tail', async () => {
    const { messages } = await render(song(), { bit_depth: 'pcm16' })
    const chunks = messages.filter((m) => m.type === 'chunk')
    const rendered = chunks.map((m) => m.rendered)
    expect(rendered).toEqual([...rendered].sort((a, b) => a - b))
    expect(chunks[0].tail).toBe(false)
    expect(chunks.at(-1)!.tail).toBe(true)
    expect(chunks.every((m) => m.body === 4 * 48_000)).toBe(true)
    expect(messages.at(-1)!.type).toBe('done')
  })

  it('honours the format and the sample rate', async () => {
    const float = wav((await render(song(), { bit_depth: 'float32', sample_rate: 44_100 })).messages)
    expect(float.view.getUint16(20, true)).toBe(3)
    expect(float.view.getUint16(34, true)).toBe(32)
    expect(float.view.getUint32(24, true)).toBe(44_100)
    expect(float.file.length).toBe(58 + float.done.written * 8)
    const small = wav((await render(song(), { bit_depth: 'pcm16' })).messages)
    expect(small.view.getUint16(34, true)).toBe(16)
  })

  it('waits for the page after each chunk and renders nothing more than it lets', async () => {
    // A gate that lets two chunks go, then holds the render until it is told: the render stops
    // there, with only what was let through posted.
    const core = song()
    const job = core.exportJob({})
    if (job.status !== 'ready') throw new Error('ready')
    const messages: FromExport[] = []
    let release: () => void = () => undefined
    let gates = 0
    const rendering = renderExport(
      {
        type: 'start',
        module: engine.slice(0),
        sampleRate: 48_000,
        bits: 24,
        bodyFrames: job.plan.body_frames,
        setup: job.setup.slice().buffer,
        start: job.start.slice().buffer,
        stop: job.stop.slice().buffer,
        folder: null,
      },
      (message) => messages.push(message),
      () => (++gates <= 2 ? Promise.resolve() : new Promise<void>((resolve) => (release = resolve))),
    )
    const until = async (done: () => boolean) => {
      for (let i = 0; i < 1000 && !done(); i++) await new Promise((resolve) => setTimeout(resolve, 2))
    }
    // Two chunks pass the gate, the third is posted and waits at it.
    await until(() => gates === 3)
    expect(gates).toBe(3)
    expect(messages.filter((m) => m.type === 'chunk')).toHaveLength(3)
    expect(messages.some((m) => m.type === 'done')).toBe(false)
    // Nothing moves while it is held.
    await new Promise((resolve) => setTimeout(resolve, 30))
    expect(messages.filter((m) => m.type === 'chunk')).toHaveLength(3)
    // One release lets one more chunk through, which waits at the gate in its turn.
    release()
    await until(() => gates === 4)
    expect(messages.filter((m) => m.type === 'chunk')).toHaveLength(4)
    // Let the rest go: the file is whole.
    for (let i = 0; i < 1000 && !messages.some((m) => m.type === 'done'); i++) {
      release()
      await new Promise((resolve) => setTimeout(resolve, 2))
    }
    await rendering.catch(() => undefined)
    expect(messages.at(-1)!.type).toBe('done')
  })

  it('is refused by the core on an empty arrangement, and posts nothing', () => {
    const core = ProjectCore.create(module, 'single_synth', 1, seed)
    const job = core.exportJob({})
    expect(job.status).toBe('refused')
    expect(core.exportJob({ region: 'loop' }).status).toBe('refused')
  })

  it('posts an error, not a file, when the engine is given a stream it refuses', async () => {
    const core = song()
    const job = core.exportJob({})
    if (job.status !== 'ready') throw new Error('ready')
    const messages: FromExport[] = []
    // A truncated setup: the stream stops, nothing is rendered.
    await renderExport(
      {
        type: 'start',
        module: engine.slice(0),
        sampleRate: 48_000,
        bits: 24,
        bodyFrames: 192_000,
        setup: job.setup.slice(0, 30).buffer,
        start: job.start.slice().buffer,
        stop: job.stop.slice().buffer,
        folder: null,
      },
      (message) => messages.push(message),
    )
    expect(messages).toHaveLength(1)
    expect(messages[0]).toMatchObject({ type: 'error' })
    // A length the engine refuses.
    const refused: FromExport[] = []
    await renderExport(
      {
        type: 'start',
        module: engine.slice(0),
        sampleRate: 48_000,
        bits: 24,
        bodyFrames: 0,
        setup: job.setup.slice().buffer,
        start: job.start.slice().buffer,
        stop: job.stop.slice().buffer,
        folder: null,
      },
      (m) => refused.push(m),
    )
    expect(refused[0]).toMatchObject({ type: 'error' })
  })
})

// #647: an audio clip, read from the project folder by the executor the live engine uses.
const MEDIA = '47'.repeat(32)
const TONE_RATE = 44_100
const TONE_SECONDS = 3

/** A mono 16-bit WAV of a 440 Hz sine at half scale. */
function toneWav(): Uint8Array {
  const frames = TONE_RATE * TONE_SECONDS
  const view = new DataView(new ArrayBuffer(44 + frames * 2))
  const text = (at: number, s: string) => [...s].forEach((c, i) => view.setUint8(at + i, c.charCodeAt(0)))
  text(0, 'RIFF')
  view.setUint32(4, 36 + frames * 2, true)
  text(8, 'WAVEfmt ')
  view.setUint32(16, 16, true)
  view.setUint16(20, 1, true)
  view.setUint16(22, 1, true)
  view.setUint32(24, TONE_RATE, true)
  view.setUint32(28, TONE_RATE * 2, true)
  view.setUint16(32, 2, true)
  view.setUint16(34, 16, true)
  text(36, 'data')
  view.setUint32(40, frames * 2, true)
  for (let f = 0; f < frames; f++) view.setInt16(44 + f * 2, Math.round(Math.sin((f * 440 * 2 * Math.PI) / TONE_RATE) * 16_384), true)
  return new Uint8Array(view.buffer)
}

/** A project folder holding `blobs/<MEDIA>`, as a directory handle gives it. */
function folderWith(blob: Uint8Array): Dir {
  const file = new File([blob.slice().buffer], MEDIA)
  const dir = (entries: Record<string, Dir | File>): Dir => ({
    getDirectoryHandle: async (name) => {
      const entry = entries[name]
      if (!entry || entry instanceof File) throw new DOMException(name, 'NotFoundError')
      return entry
    },
    getFileHandle: async (name) => {
      const entry = entries[name]
      if (!(entry instanceof File)) throw new DOMException(name, 'NotFoundError')
      return { getFile: async () => entry }
    },
  })
  return dir({ blobs: dir({ [MEDIA]: file }) })
}

/** A core whose audio track has the tone placed at the start of the arrangement. */
function audioSong(): ProjectCore {
  const core = ProjectCore.create(module, 'single_synth', 1, seed)
  const created = core.run({ type: 'track.create', kind: 'audio' })
  if (created.report.status !== 'applied' || created.report.output.type !== 'track.create') throw new Error(JSON.stringify(created.report))
  const track = created.report.output.track
  core.stageMedia({ hash: MEDIA, sample_rate: TONE_RATE, channels: 1, frames: TONE_RATE * TONE_SECONDS, name: 'tone.wav' })
  const placed = core.run({ type: 'clip.import_audio', track, media: MEDIA, at: { at: 'arrangement', start: 0 } })
  if (placed.report.status !== 'applied') throw new Error(JSON.stringify(placed.report))
  return core
}

describe('the export of audio clips (#647)', () => {
  it('reads the clip from the project folder and renders it at its level, waiting for the reads', async () => {
    let gates = 0
    const { job, messages } = await render(audioSong(), { bit_depth: 'pcm16' }, async () => void gates++, folderWith(toneWav()))
    const { done, view } = wav(messages)
    expect(done.missing).toEqual([])
    // Three seconds of the tone, resampled to 48 kHz.
    expect(job.plan.body_frames).toBe(3 * 48_000)
    const sample = (frame: number) => Math.abs(view.getInt16(44 + frame * 4, true) / 32_768)
    const peak = (from: number, to: number) => Math.max(...Array.from({ length: to - from }, (_, i) => sample(from + i)))
    // At half scale from its first milliseconds to its end, past the handover from the preload.
    for (const second of [0.1, 1, 2, 2.9]) expect(peak(second * 48_000, second * 48_000 + 960)).toBeCloseTo(0.5, 1)
    // Only the chunks with bytes, the ones the page acknowledges, take room.
    const full = messages.filter((m) => m.type === 'chunk' && m.bytes.byteLength > 0)
    expect(gates).toBe(full.length)
    expect(messages.some((m) => m.type === 'chunk' && m.bytes.byteLength === 0)).toBe(true)
  })

  it('renders a clip whose file is missing as silence and names its media', async () => {
    const { messages } = await render(audioSong(), { bit_depth: 'pcm16' }, undefined, null)
    const { done } = wav(messages)
    expect(done.missing).toEqual([MEDIA])
    expect(done.peak).toBe(0)
  })
})
