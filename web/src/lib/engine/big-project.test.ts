// A big project's snapshot through the page's own path (#245): the project core (project.wasm)
// builds a project of thousands of clips by commands and encodes its setup, its opening from a
// folder and the snapshot of a switch back to Web; the prepare Worker's queue (`PushQueue`)
// sends each to the shipped engine module (engine.wasm, prepare and render instances on one
// shared memory, as the Worker and the AudioWorklet run it) in pieces as the render consumes it.
// Nothing is refused, the render instance never allocates, and the project plays. The time to a
// playable state (render quanta and wall time) is printed.
import { readFileSync } from 'node:fs'
import { beforeAll, describe, expect, it } from 'vitest'
import { SINGLE_SYNTH, type Command } from '../generated/protocol'
import { coreModule, ProjectCore, type FolderEntry } from '../project/core'
import { becomeRenderInstance, createChannel, instantiate, MAX_FRAMES, ROLE_RENDER, type EngineExports } from './engine-abi'
import { ENGINE_BUSY, PushQueue, startsWithReset } from './push-queue'

const engineModule = new WebAssembly.Module(new Uint8Array(readFileSync(new URL('./wasm/engine.wasm', import.meta.url))).buffer)
let coreWasm: WebAssembly.Module

beforeAll(async () => {
  coreWasm = await coreModule(readFileSync(new URL('../project/wasm/project.wasm', import.meta.url)))
})

const QUANTUM = 128
const RATE = 48_000
const BAR = 3840
/** What the engine takes before it is busy: its rt ring and its prepare queue (`engine::msg::limits`). */
const AT_ONCE = 4096 + 4096
const seed = { timeMs: 1_760_000_000_000, random: [1, 2, 3] as [number, number, number] }

function run(core: ProjectCore, command: Command) {
  const { report, engine } = core.run(command)
  if (report.status !== 'applied') throw new Error(JSON.stringify(report))
  return { output: report.output, engine }
}

/** 48 tracks of 64 grid clips and 64 arrangement clips of one note each (6 144 clips), by commands. */
function bigCore(): ProjectCore {
  const core = ProjectCore.create(coreWasm, 'single_synth', 1, seed)
  const track = SINGLE_SYNTH.track
  const scenes: string[] = []
  for (let i = 0; i < 64; i++) {
    const { output } = run(core, { type: 'scene.create' })
    if (output.type !== 'scene.create') throw new Error('a scene')
    scenes.push(output.scene)
  }
  const targets = [
    ...scenes.map((scene) => ({ at: 'slot' as const, scene })),
    ...Array.from({ length: 64 }, (_, bar) => ({ at: 'arrangement' as const, start: bar * BAR })),
  ]
  for (const at of targets) {
    const { output } = run(core, { type: 'clip.create', track, at, length: BAR })
    if (output.type !== 'clip.create') throw new Error('a clip')
    run(core, { type: 'clip.notes.add', clip: output.clip, notes: [{ start: 0, pitch: 60, length: BAR / 2, velocity: 100, cents: 0, chance: 100, repeat: 1 }] })
  }
  for (let i = 1; i < 48; i++) run(core, { type: 'track.duplicate', track })
  return core
}

function clipsOf(core: ProjectCore): number {
  return Object.values(core.project().project.tracks).reduce((n, t) => n + Object.keys(t?.clips ?? {}).length, 0)
}

/** Messages in a stream (the 16-byte header's length, frames aligned on 4 bytes). */
function count(bytes: Uint8Array): number {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength)
  let n = 0
  for (let at = 0; at + 16 <= bytes.length; n++) at += 16 + ((view.getUint32(at + 8, true) + 3) & ~3)
  return n
}

/** An engine as the page runs it: the prepare side behind the Worker's queue, the render side. */
function engine() {
  const memory = new WebAssembly.Memory({ initial: 256, maximum: 65536, shared: true })
  const prepare = instantiate(engineModule, memory)
  const setup = createChannel(prepare, RATE)
  const render = instantiate(engineModule, memory)
  becomeRenderInstance(render, setup)
  const ch = setup.channel
  const stopped: number[] = []
  const queue = new PushQueue(prepare, memory, ch, (code) => stopped.push(code))
  let peak = 0
  /** One render quantum (the AudioWorklet), then the Worker's tick. */
  const quantum = () => {
    const n = render.fk_render(ch, QUANTUM)
    const left = new Float32Array(memory.buffer, render.fk_left(ch), MAX_FRAMES)
    for (let i = 0; i < n; i++) peak = Math.max(peak, Math.abs(left[i]))
    queue.flush()
  }
  /** Quanta until the engine applied everything: nothing waits nor is pending and a quantum applied nothing. */
  const untilApplied = () => {
    let quanta = 0
    for (;;) {
      const before = render.fk_host_counter(ch, 0)
      quantum()
      if (render.fk_host_counter(ch, 0) === before && queue.waiting === 0 && prepare.fk_prepare_counter(ch, 1) === 0) return quanta
      quanta++
      if (quanta > 10_000) throw new Error('the queue does not drain')
    }
  }
  const allocs = () => [render.fk_allocs(ROLE_RENDER), render.fk_frees(ROLE_RENDER)]
  return {
    prepare: prepare as EngineExports,
    render,
    ch,
    queue,
    stopped,
    quantum,
    untilApplied,
    allocs,
    peakSince: () => {
      const p = peak
      peak = 0
      return p
    },
    /** Rejected (prepare), skipped (render). */
    refused: () => [prepare.fk_prepare_counter(ch, 0), render.fk_host_counter(ch, 1)],
  }
}

/** Launches the first scene and renders half a second: the peak. */
function playFirstScene(core: ProjectCore, e: ReturnType<typeof engine>): number {
  const scene = Object.keys(core.project().project.scenes)[0]
  e.queue.push(run(core, { type: 'launch.scene', scene }).engine)
  e.peakSince()
  for (let q = 0; q < (RATE / QUANTUM) * 0.5; q++) e.quantum()
  return e.peakSince()
}

function report(what: string, bytes: Uint8Array, quanta: number, ms: number) {
  const audio = ((quanta * QUANTUM) / RATE) * 1000
  console.log(
    `${what}: ${count(bytes)} messages, ${bytes.length} bytes; applied after ${quanta} quanta = ${audio.toFixed(0)} ms of audio at ${RATE} Hz; ${ms.toFixed(0)} ms of wall time under Node`,
  )
}

describe('a big project through the page', () => {
  it('starts: the setup goes in pieces, nothing is refused, the render does not allocate, the project plays', () => {
    const core = bigCore()
    expect(clipsOf(core)).toBeGreaterThanOrEqual(6000)
    const { report: setupReport, engine: bytes } = core.setup()
    expect(setupReport.status).toBe('encoded')
    expect(count(bytes)).toBeGreaterThan(AT_ONCE)
    const e = engine()
    // A first quantum outside the count: the render instance's first block may set itself up.
    e.quantum()
    const allocs = e.allocs()
    const start = performance.now()
    e.queue.push(bytes)
    expect(e.queue.waiting).toBe(1)
    const quanta = e.untilApplied()
    report('setup', bytes, quanta, performance.now() - start)
    expect(e.allocs()).toEqual(allocs)
    expect(e.stopped).toEqual([])
    expect(e.refused()).toEqual([0, 0])
    expect(playFirstScene(core, e)).toBeGreaterThan(0.01)
  }, 120_000)

  it('opens it from its folder over a project that plays', () => {
    const big = bigCore()
    const frame = big.save('everything')
    if (frame.status !== 'ok') throw new Error(frame.message)
    const entries: FolderEntry[] = frame.entries.flatMap((entry) => (entry.type === 'write' ? [{ path: entry.path, bytes: entry.bytes }] : []))
    const core = ProjectCore.create(coreWasm, 'single_synth', 1, seed)
    const e = engine()
    e.queue.push(core.setup().engine)
    e.untilApplied()
    // The project that plays: a key held on its synth.
    e.queue.push(run(core, { type: 'live.note_on', track: SINGLE_SYNTH.track, note: 60, velocity: 100 }).engine)
    for (let q = 0; q < 20; q++) e.quantum()
    expect(e.peakSince()).toBeGreaterThan(0.01)
    core.stage('big', entries)
    const { report: opened, engine: bytes } = core.open('big')
    expect(opened.status).toBe('applied')
    expect(clipsOf(core)).toBe(clipsOf(big))
    expect(count(bytes)).toBeGreaterThan(AT_ONCE)
    const start = performance.now()
    e.queue.push(bytes)
    const quanta = e.untilApplied()
    report('project.open', bytes, quanta, performance.now() - start)
    expect(e.stopped).toEqual([])
    expect(e.refused()).toEqual([0, 0])
    expect(playFirstScene(core, e)).toBeGreaterThan(0.01)
  }, 120_000)

  it('switches back to Web: the snapshot is pushed while the audio context is suspended and goes in once it runs', () => {
    const core = bigCore()
    const e = engine()
    e.queue.push(core.setup().engine)
    e.untilApplied()
    // Boost played; back to Web, the snapshot comes before the context resumes: nothing renders.
    const { report: snap, engine: bytes } = core.snapshot()
    expect(snap.status).toBe('encoded')
    e.queue.push(bytes)
    const waiting = e.queue.waitingBytes
    for (let i = 0; i < 100; i++) e.queue.flush()
    expect(e.queue.waitingBytes).toBe(waiting)
    // A command sent meanwhile waits behind it.
    const scene = Object.keys(core.project().project.scenes)[0]
    e.queue.push(run(core, { type: 'launch.scene', scene }).engine)
    expect(e.queue.waiting).toBe(2)
    const start = performance.now()
    const quanta = e.untilApplied()
    report('switch (snapshot into a suspended context)', bytes, quanta, performance.now() - start)
    expect(e.stopped).toEqual([])
    expect(e.refused()).toEqual([0, 0])
    e.peakSince()
    for (let q = 0; q < (RATE / QUANTUM) * 0.5; q++) e.quantum()
    expect(e.peakSince()).toBeGreaterThan(0.01)
  }, 120_000)

  it('refuses a stream past the bound behind the one the engine takes, and never that one', () => {
    const core = bigCore()
    const memory = new WebAssembly.Memory({ initial: 256, maximum: 65536, shared: true })
    const prepare = instantiate(engineModule, memory)
    const ch = createChannel(prepare, RATE).channel
    const stopped: number[] = []
    const bound = 16 * 1024
    const queue = new PushQueue(prepare, memory, ch, (code) => stopped.push(code), bound)
    const setup = core.setup().engine
    queue.push(setup)
    // Nothing renders: the front stream waits whole beyond the bound, and is not refused.
    expect(queue.waitingBytes).toBeGreaterThan(bound)
    expect(stopped).toEqual([])
    const small = run(core, { type: 'transport.set_tempo', bpm: 100 }).engine
    queue.push(small)
    expect(queue.waiting).toBe(2)
    queue.push(new Uint8Array(bound))
    expect(stopped).toEqual([ENGINE_BUSY])
    expect(queue.waiting).toBe(2)
  }, 120_000)

  it('drops what waits when a stream starts the engine over, and plays the new one', () => {
    const core = bigCore()
    const e = engine()
    e.queue.push(core.setup().engine)
    e.queue.push(run(core, { type: 'transport.set_tempo', bpm: 100 }).engine)
    expect(e.queue.waiting).toBe(2)
    // A switch (or an open) while the setup still goes in: its `Reset` would wipe what waits.
    const { engine: bytes } = core.snapshot()
    expect(startsWithReset(bytes)).toBe(true)
    e.queue.push(bytes)
    expect(e.queue.waiting).toBe(1)
    e.untilApplied()
    expect(e.stopped).toEqual([])
    expect(e.refused()).toEqual([0, 0])
    expect(playFirstScene(core, e)).toBeGreaterThan(0.01)
  }, 120_000)
})
