// The export's controller (issue #297) against stand-ins for the dialog, the file and the Worker:
// its states, what it asks the core for, the order of the file's bytes, cancelling at every
// point, and every way it can fail.
import { describe, expect, it, vi } from 'vitest'
import type { Command } from '../generated/protocol'
import type { ExportJob } from '../project/core'
import {
  AudioExport,
  fileNameOf,
  headerLength,
  type ExportDeps,
  type ExportHost,
  type ExportSink,
  type ExportWorkerPort,
  type SinkChoice,
} from './audio-export.svelte'
import type { FromExport, ToExport, ToStart } from './export-render'

const PLAN = {
  settings: { region: 'song', bit_depth: 'pcm24', sample_rate: 48_000 },
  start_tick: 0,
  end_tick: 7680,
  body_frames: 192_000,
  duration_secs: 4,
} as const

function ready(plan: Record<string, unknown> = {}): ExportJob {
  return { status: 'ready', plan: { ...PLAN, ...plan } as never, setup: Uint8Array.of(1, 2, 3), stop: Uint8Array.of(4) }
}

class FakeSink implements ExportSink {
  readonly kind = 'file' as const
  written: number[][] = []
  header: number[] | null = null
  aborted = false
  failWrite = false
  failFinish = false
  constructor(readonly name = 'Song.wav') {}
  async write(bytes: Uint8Array) {
    if (this.failWrite) throw new Error('disk full')
    this.written.push([...bytes])
  }
  async finish(header: Uint8Array) {
    if (this.failFinish) throw new Error('close failed')
    this.header = [...header]
  }
  async abort() {
    this.aborted = true
  }
}

class FakeWorker implements ExportWorkerPort {
  onmessage: ((event: MessageEvent<FromExport>) => void) | null = null
  onerror: ((event: ErrorEvent) => void) | null = null
  terminated = false
  started: ToStart | null = null
  /** The acknowledgements of written chunks the Worker received. */
  acks = 0
  postMessage(message: ToExport) {
    if (message.type === 'start') this.started = message
    else this.acks++
  }
  terminate() {
    this.terminated = true
  }
  say(message: FromExport) {
    this.onmessage?.({ data: message } as MessageEvent<FromExport>)
  }
}

function setup(
  overrides: {
    job?: ExportJob | null
    pick?: SinkChoice
    hasContent?: boolean
    hasLoop?: boolean
    module?: Promise<ArrayBuffer>
    refuse?: boolean
    canChoose?: boolean
  } = {},
) {
  const sink = new FakeSink()
  const worker = new FakeWorker()
  let clock = 1000
  // The dispatcher runs the command and, when the project accepts it, tells the controller (an effect).
  const commands: Command[] = []
  const host: ExportHost = {
    run: vi.fn((command: Command) => {
      commands.push(command)
      if (overrides.refuse || command.type !== 'project.export_audio') return
      exporter.request({ region: command.region ?? undefined, bit_depth: command.bit_depth ?? undefined, sample_rate: command.sample_rate ?? undefined })
    }),
    job: vi.fn(() => (overrides.job === undefined ? ready() : overrides.job)),
    projectName: () => 'My: song?',
    hasContent: () => overrides.hasContent ?? true,
    hasLoop: () => overrides.hasLoop ?? false,
    songBars: () => 4,
    folder: async () => null,
  }
  const deps: ExportDeps = {
    pick: vi.fn(async () => overrides.pick ?? ({ status: 'chosen', sink } as SinkChoice)),
    spawn: () => worker,
    engineModule: () => overrides.module ?? Promise.resolve(new ArrayBuffer(8)),
    canChoose: overrides.canChoose ?? true,
    now: () => (clock += 250),
  }
  const exporter: AudioExport = new AudioExport(host, deps)
  return { exporter, host, deps, sink, worker, commands }
}

const chunk = (rendered: number, bytes: number[], tail = false): FromExport => ({
  type: 'chunk',
  bytes: Uint8Array.from(bytes).buffer,
  rendered,
  body: 192_000,
  tail,
})
const done = (over: Partial<Extract<FromExport, { type: 'done' }>> = {}): FromExport => ({
  type: 'done',
  header: Uint8Array.of(9, 9).buffer,
  written: 240_000,
  peak: 0.5,
  clipped: 0,
  tailCut: false,
  ...over,
})
const flush = () => new Promise((resolve) => setTimeout(resolve, 0))

describe('the export dialog', () => {
  it('opens with the defaults, or with what a command asked for', () => {
    const { exporter } = setup({ hasLoop: true })
    exporter.show()
    expect(exporter.open).toBe(true)
    expect(exporter.phase).toBe('options')
    expect(exporter.choices).toMatchObject({ region: 'song', bitDepth: 'pcm24', sampleRate: 48_000 })
    exporter.request({ region: 'loop', bit_depth: 'float32', sample_rate: 96_000 })
    expect(exporter.choices).toMatchObject({ region: 'loop', bitDepth: 'float32', sampleRate: 96_000 })
    exporter.request({})
    expect(exporter.choices).toMatchObject({ region: 'song', bitDepth: 'pcm24', sampleRate: 48_000 })
  })

  it('does not offer a loop that does not exist', () => {
    const { exporter } = setup({ hasLoop: false })
    exporter.request({ region: 'loop' })
    expect(exporter.choices.region).toBe('song')
  })

  it('names the file after the project, without what a file system refuses', () => {
    expect(fileNameOf('My: song?')).toBe('My- song-.wav')
    expect(fileNameOf('  ..  ')).toBe('Faulk.wav')
    expect(fileNameOf('a/b\\c')).toBe('a-b-c.wav')
    expect(fileNameOf('x'.repeat(300))).toHaveLength(104)
    expect(headerLength('pcm24')).toBe(44)
    expect(headerLength('float32')).toBe(58)
  })
})

describe('an export', () => {
  it('asks the core, asks for the file, renders, writes the bytes in order, then the header', async () => {
    const { exporter, host, deps, sink, worker, commands } = setup()
    exporter.show()
    exporter.start()
    await flush()
    // The button runs the command, as any other caller does; its effect begins the render.
    expect(commands).toEqual([{ type: 'project.export_audio', region: 'song', bit_depth: 'pcm24', sample_rate: 48_000 }])
    expect(host.job).toHaveBeenCalledWith({ region: 'song', bit_depth: 'pcm24', sample_rate: 48_000 })
    // The file is asked for first, with room for the header.
    expect(deps.pick).toHaveBeenCalledWith('My- song-.wav', 44, null)
    expect(exporter.phase).toBe('rendering')
    expect(worker.started).toMatchObject({ type: 'start', sampleRate: 48_000, bits: 24, bodyFrames: 192_000 })
    expect(new Uint8Array(worker.started!.setup)).toEqual(Uint8Array.of(1, 2, 3))
    expect(new Uint8Array(worker.started!.stop)).toEqual(Uint8Array.of(4))
    worker.say(chunk(96_000, [1, 2]))
    expect(exporter.fraction).toBeCloseTo(0.5)
    expect(exporter.tail).toBe(false)
    worker.say(chunk(192_000, [3], true))
    expect(exporter.fraction).toBe(1)
    expect(exporter.tail).toBe(true)
    worker.say(done())
    await flush()
    expect(sink.written).toEqual([[1, 2], [3]])
    expect(sink.header).toEqual([9, 9])
    expect(exporter.phase).toBe('done')
    expect(worker.terminated).toBe(true)
    expect(exporter.result).toMatchObject({
      name: 'Song.wav',
      kind: 'file',
      bytes: 44 + 3,
      seconds: 5,
      peakDb: expect.closeTo(-6.02, 1),
      clipped: 0,
      tailCut: false,
    })
    expect(exporter.result!.renderMs).toBeGreaterThan(0)
  })

  it('says when the level clipped, when the tail was cut, and when the file is silent', async () => {
    const { exporter, worker } = setup()
    exporter.show()
    exporter.start()
    await flush()
    worker.say(done({ clipped: 12, tailCut: true, peak: 0 }))
    await flush()
    expect(exporter.result).toMatchObject({ clipped: 12, tailCut: true, peakDb: -Infinity })
  })

  it('stays in the options when the user leaves the file dialog', async () => {
    const { exporter, worker } = setup({ pick: { status: 'cancelled' } })
    exporter.show()
    exporter.start()
    await flush()
    expect(exporter.phase).toBe('options')
    expect(worker.started).toBeNull()
  })

  it('fails, and says so, when the browser refuses the file', async () => {
    const { exporter } = setup({ pick: { status: 'refused' } })
    exporter.show()
    exporter.start()
    await flush()
    expect(exporter.phase).toBe('failed')
    expect(exporter.error).toBe('export.noAccess')
  })

  it('fails when the project refuses the command, and asks for no file', async () => {
    const { exporter, deps, worker } = setup({ refuse: true })
    exporter.show()
    exporter.start()
    await flush()
    expect(exporter.phase).toBe('failed')
    expect(exporter.error).toBe('export.refused')
    expect(deps.pick).not.toHaveBeenCalled()
    expect(worker.started).toBeNull()
    // The next export is not mistaken for the refused one.
    exporter.request({ bit_depth: 'pcm16' })
    expect(exporter.phase).toBe('options')
    expect(exporter.choices.bitDepth).toBe('pcm16')
  })

  it('an agent’s command only opens the dialog: no file is asked for without a click', async () => {
    const { exporter, deps } = setup()
    exporter.request({ region: 'song', bit_depth: 'float32' })
    await flush()
    expect(exporter.open).toBe(true)
    expect(exporter.phase).toBe('options')
    expect(deps.pick).not.toHaveBeenCalled()
  })

  it('fails when the core refuses the project or cannot encode it, before asking for a file', async () => {
    for (const job of [
      { status: 'refused', error: { code: 'invalid', field: 'region', message: 'empty' } } as unknown as ExportJob,
      { status: 'failed', stream_error: -8 } as ExportJob,
      null,
    ]) {
      const { exporter, deps } = setup({ job })
      exporter.show()
      exporter.start()
      await flush()
      expect(exporter.phase).toBe('failed')
      expect(deps.pick).not.toHaveBeenCalled()
    }
  })

  it('fails and drops the file when the Worker fails', async () => {
    const { exporter, sink, worker } = setup()
    exporter.show()
    exporter.start()
    await flush()
    worker.say(chunk(1000, [1]))
    worker.say({ type: 'error', message: 'boom', code: -100 })
    await flush()
    expect(exporter.phase).toBe('failed')
    expect(exporter.error).toBe('export.failed')
    expect(worker.terminated).toBe(true)
    expect(sink.aborted).toBe(true)
    expect(sink.header).toBeNull()
  })

  it('fails and drops the file when the Worker cannot start', async () => {
    const { exporter, sink, worker } = setup()
    exporter.show()
    exporter.start()
    await flush()
    worker.onerror?.({ message: 'no module' } as ErrorEvent)
    await flush()
    expect(exporter.phase).toBe('failed')
    expect(sink.aborted).toBe(true)
  })

  it('fails and stops rendering when a write fails (a full disk)', async () => {
    const { exporter, sink, worker } = setup()
    sink.failWrite = true
    exporter.show()
    exporter.start()
    await flush()
    worker.say(chunk(1000, [1]))
    await flush()
    expect(exporter.phase).toBe('failed')
    expect(exporter.error).toBe('export.writeFailed')
    expect(worker.terminated).toBe(true)
    expect(sink.aborted).toBe(true)
    // What the dead Worker would still say changes nothing.
    worker.say(done())
    await flush()
    expect(exporter.phase).toBe('failed')
  })

  it('fails when the file cannot be completed', async () => {
    const { exporter, sink, worker } = setup()
    sink.failFinish = true
    exporter.show()
    exporter.start()
    await flush()
    worker.say(done())
    await flush()
    expect(exporter.phase).toBe('failed')
    expect(exporter.error).toBe('export.writeFailed')
    expect(sink.aborted).toBe(true)
  })

  it('fails when the engine module cannot be fetched', async () => {
    const { exporter, sink } = setup({ module: Promise.reject(new Error('offline')) })
    exporter.show()
    exporter.start()
    await flush()
    expect(exporter.phase).toBe('failed')
    expect(sink.aborted).toBe(true)
  })

  it('cancels while rendering: the Worker is terminated, the file dropped, nothing more is heard', async () => {
    const { exporter, sink, worker } = setup()
    exporter.show()
    exporter.start()
    await flush()
    worker.say(chunk(50_000, [1]))
    exporter.cancel()
    await flush()
    expect(exporter.phase).toBe('options')
    expect(exporter.fraction).toBe(0)
    expect(worker.terminated).toBe(true)
    expect(sink.aborted).toBe(true)
    worker.say(chunk(100_000, [2]))
    worker.say(done())
    await flush()
    expect(exporter.phase).toBe('options')
    expect(exporter.result).toBeNull()
    expect(sink.header).toBeNull()
  })

  it('cancels while the file dialog is open: the file chosen afterwards is dropped', async () => {
    let choose: (choice: SinkChoice) => void = () => undefined
    const { exporter, deps, sink, worker } = setup()
    deps.pick = vi.fn(() => new Promise<SinkChoice>((resolve) => (choose = resolve)))
    exporter.show()
    exporter.start()
    await flush()
    expect(exporter.phase).toBe('picking')
    exporter.cancel()
    choose({ status: 'chosen', sink })
    await flush()
    expect(exporter.phase).toBe('options')
    expect(sink.aborted).toBe(true)
    expect(worker.started).toBeNull()
  })

  it('cancels while the project folder is awaited (a save under way): the file dialog never opens', async () => {
    let known: (folder: null) => void = () => undefined
    const { exporter, deps, host } = setup()
    host.folder = () => new Promise((resolve) => (known = resolve))
    deps.pick = vi.fn()
    exporter.show()
    exporter.start()
    await flush()
    exporter.cancel()
    known(null)
    await flush()
    expect(deps.pick).not.toHaveBeenCalled()
    expect(exporter.phase).toBe('options')
  })

  it('closing the dialog cancels a render under way', async () => {
    const { exporter, worker } = setup()
    exporter.show()
    exporter.start()
    await flush()
    exporter.close()
    expect(exporter.open).toBe(false)
    expect(worker.terminated).toBe(true)
  })

  it('acknowledges each chunk once it is written, never after a failure', async () => {
    const { exporter, sink, worker } = setup()
    exporter.show()
    exporter.start()
    await flush()
    worker.say(chunk(1000, [1]))
    worker.say(chunk(2000, [2]))
    await flush()
    expect(sink.written).toEqual([[1], [2]])
    expect(worker.acks).toBe(2)
    sink.failWrite = true
    worker.say(chunk(3000, [3]))
    await flush()
    expect(exporter.phase).toBe('failed')
    expect(worker.acks).toBe(2)
  })

  it('exports bars: the command carries them, the dialog checks them, the core is asked for them', async () => {
    const { exporter, host, commands } = setup()
    exporter.show()
    // The dialog starts on the whole song: from bar 1 to the last bar.
    expect(exporter.choices).toMatchObject({ region: 'song', fromBar: 1, toBar: 4 })
    exporter.choices.region = 'bars'
    exporter.choices.fromBar = 2
    exporter.choices.toBar = 3
    expect(exporter.barsValid).toBe(true)
    expect(exporter.canStart).toBe(true)
    exporter.start()
    await flush()
    expect(commands).toEqual([{ type: 'project.export_audio', region: 'bars', bit_depth: 'pcm24', sample_rate: 48_000, from_bar: 2, to_bar: 3 }])
    expect(host.job).toHaveBeenCalledWith({ region: 'bars', bit_depth: 'pcm24', sample_rate: 48_000, from_bar: 2, to_bar: 3 })
    // An agent's command brings its bars into the dialog.
    exporter.close()
    exporter.request({ region: 'bars', from_bar: 3, to_bar: 5 })
    expect(exporter.choices).toMatchObject({ region: 'bars', fromBar: 3, toBar: 5 })
    // Bars that make no range cannot be exported, and none is sent for the song.
    for (const [from, to] of [
      [0, 2],
      [3, 2],
      [1.5, 2],
      [1, Number.NaN],
    ]) {
      exporter.choices.fromBar = from
      exporter.choices.toBar = to
      expect(exporter.canStart, `${from} to ${to}`).toBe(false)
    }
    exporter.choices.region = 'song'
    expect(exporter.canStart).toBe(true)
  })

  it('says why a range is too long, from what the core answers', async () => {
    const tooLong = { status: 'refused', error: { code: 'limit', entity: 'export seconds', max: 1800 } } as unknown as ExportJob
    const { exporter, host } = setup({ refuse: true, job: tooLong })
    exporter.show()
    exporter.start()
    await flush()
    expect(host.job).toHaveBeenCalled()
    expect(exporter.error).toBe('export.tooLong')
    // The same refusal met when the core is asked for the render.
    const again = setup({ job: tooLong })
    again.exporter.show()
    again.exporter.start()
    await flush()
    expect(again.exporter.error).toBe('export.tooLong')
  })

  it('refuses a download too big to keep in memory, and writes a file of any size', async () => {
    // 20 minutes at 48 kHz, 24 bits: 346 MB; at 96 kHz float32: 1.8 GB.
    const big = (frames: number) => ready({ body_frames: frames })
    const small = setup({ job: big(20 * 60 * 48_000), canChoose: false })
    small.exporter.show()
    small.exporter.start()
    await flush()
    expect(small.exporter.phase).toBe('rendering')
    const huge = setup({ job: big(20 * 60 * 48_000), canChoose: false })
    huge.exporter.show({ bitDepth: 'float32', sampleRate: 96_000 })
    huge.exporter.start()
    await flush()
    expect(huge.exporter.phase).toBe('failed')
    expect(huge.exporter.error).toBe('export.tooBig')
    expect(huge.deps.pick).not.toHaveBeenCalled()
    // With a file dialog the same song goes to the file as it is made.
    const file = setup({ job: big(20 * 60 * 96_000) })
    file.exporter.show({ bitDepth: 'float32', sampleRate: 96_000 })
    file.exporter.start()
    await flush()
    expect(file.exporter.phase).toBe('rendering')
  })

  it('does not start a second export while one runs, and a request does not reset it', async () => {
    const { exporter, host } = setup()
    exporter.show()
    exporter.start()
    await flush()
    exporter.start()
    await flush()
    expect(host.job).toHaveBeenCalledTimes(1)
    exporter.request({ bit_depth: 'pcm16' })
    expect(exporter.phase).toBe('rendering')
    expect(exporter.choices.bitDepth).toBe('pcm24')
  })

  it('a new export after a finished one starts from a clean dialog', async () => {
    const { exporter, worker } = setup()
    exporter.show()
    exporter.start()
    await flush()
    worker.say(done())
    await flush()
    expect(exporter.phase).toBe('done')
    exporter.show()
    expect(exporter.phase).toBe('options')
    expect(exporter.result).toBeNull()
  })
})
