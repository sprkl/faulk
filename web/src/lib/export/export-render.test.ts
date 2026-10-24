// The export's render (issue #297) through the shipped engine module and the project core, both
// built by `pnpm test`, under Node: a song made by commands is exported as the Worker does it,
// and the file is read back.
import { readFileSync } from 'node:fs'
import { beforeAll, describe, expect, it } from 'vitest'
import { SINGLE_SYNTH, type Command } from '../generated/protocol'
import { coreModule, ProjectCore, type ExportRequest } from '../project/core'
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
function render(core: ProjectCore, request: ExportRequest) {
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
    stop: job.stop.slice().buffer,
  }
  renderExport(start, (message) => messages.push(message))
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
  it('writes a WAV file of the song: its header, its length, its level', () => {
    const { job, messages } = render(song(), {})
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
  })

  it('reports its progress up to the end of the song, then the tail', () => {
    const { messages } = render(song(), { bit_depth: 'pcm16' })
    const chunks = messages.filter((m) => m.type === 'chunk')
    const rendered = chunks.map((m) => m.rendered)
    expect(rendered).toEqual([...rendered].sort((a, b) => a - b))
    expect(chunks[0].tail).toBe(false)
    expect(chunks.at(-1)!.tail).toBe(true)
    expect(chunks.every((m) => m.body === 4 * 48_000)).toBe(true)
    expect(messages.at(-1)!.type).toBe('done')
  })

  it('honours the format and the sample rate', () => {
    const float = wav(render(song(), { bit_depth: 'float32', sample_rate: 44_100 }).messages)
    expect(float.view.getUint16(20, true)).toBe(3)
    expect(float.view.getUint16(34, true)).toBe(32)
    expect(float.view.getUint32(24, true)).toBe(44_100)
    expect(float.file.length).toBe(58 + float.done.written * 8)
    const small = wav(render(song(), { bit_depth: 'pcm16' }).messages)
    expect(small.view.getUint16(34, true)).toBe(16)
  })

  it('is refused by the core on an empty arrangement, and posts nothing', () => {
    const core = ProjectCore.create(module, 'single_synth', 1, seed)
    const job = core.exportJob({})
    expect(job.status).toBe('refused')
    expect(core.exportJob({ region: 'loop' }).status).toBe('refused')
  })

  it('posts an error, not a file, when the engine is given a stream it refuses', () => {
    const core = song()
    const job = core.exportJob({})
    if (job.status !== 'ready') throw new Error('ready')
    const messages: FromExport[] = []
    // A truncated setup: the stream stops, nothing is rendered.
    renderExport(
      { type: 'start', module: engine.slice(0), sampleRate: 48_000, bits: 24, bodyFrames: 192_000, setup: job.setup.slice(0, 30).buffer, stop: job.stop.slice().buffer },
      (message) => messages.push(message),
    )
    expect(messages).toHaveLength(1)
    expect(messages[0]).toMatchObject({ type: 'error' })
    // A length the engine refuses.
    const refused: FromExport[] = []
    renderExport({ type: 'start', module: engine.slice(0), sampleRate: 48_000, bits: 24, bodyFrames: 0, setup: job.setup.slice().buffer, stop: job.stop.slice().buffer }, (m) => refused.push(m))
    expect(refused[0]).toMatchObject({ type: 'error' })
  })
})
