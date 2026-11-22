// The helper's import in Boost (#778), the page's side against a stand-in port: the job sent, progress, the last word of
// the helper (done, refusal by its code), a stop that still waits for that last word, the helper gone or silent.
import { describe, expect, it, vi } from 'vitest'
import type { HelperImportMessage } from '../../engine/engine'
import { ImportError } from './abi'
import {
  chooseWithHelper,
  failureCode,
  HELPER_IMPORT_SILENCE_MS,
  runHelperImport,
  type HelperImportPort,
  type HelperImportStart,
  type Timers,
} from './helper-import'

function port() {
  const listeners = new Set<(m: HelperImportMessage | null) => void>()
  const started: HelperImportStart[] = []
  const cancelled: number[] = []
  const chosen: number[] = []
  const p: HelperImportPort = {
    project: '01J0000000000000000000000A',
    choose: (id) => chosen.push(id),
    start: (start) => started.push(start),
    cancel: (id) => cancelled.push(id),
    listen: (l) => {
      listeners.add(l)
      return () => listeners.delete(l)
    },
  }
  const say = (m: HelperImportMessage | null) => [...listeners].forEach((l) => l(m))
  return { p, started, cancelled, chosen, say, listeners }
}

/** Timers driven by hand: `fire()` runs the one set. */
function manualTimers() {
  let pending: (() => void) | null = null
  const timers: Timers = {
    set: (f) => {
      pending = f
      return f
    },
    clear: (h) => {
      if (pending === h) pending = null
    },
  }
  return { timers, fire: () => pending?.(), armed: () => pending !== null }
}

const done = (id: number) => ({
  type: 'import.done' as const,
  id,
  hash: 'ab'.repeat(32),
  sample_rate: 48_000,
  channels: 2,
  frames: 96_000,
  name: '  kick.wav ',
  direct: false,
  created: true,
  peaks: true,
})

describe("the helper's import", () => {
  it('sends the job for the project the helper has open, tells the progress, and resolves with the media the page records', async () => {
    const { p, started, say, listeners } = port()
    const progress: number[] = []
    const running = runHelperImport(p, { kind: 'folder', path: 'imports/kick.wav' }, (f) => progress.push(f), new AbortController().signal)
    expect(started).toHaveLength(1)
    const id = started[0]!.id
    expect(started[0]).toEqual({ id, project: p.project, source: { kind: 'folder', path: 'imports/kick.wav' } })
    // Another import's messages are not this one's.
    say({ type: 'import.progress', id: id + 100, fraction: 0.9 })
    say({ type: 'import.progress', id, fraction: 0.25 })
    say(done(id))
    expect(await running).toEqual({
      media: { hash: 'ab'.repeat(32), sample_rate: 48_000, channels: 2, frames: 96_000, name: 'kick.wav' },
      created: true,
      peaks: true,
    })
    expect(progress).toEqual([0.25])
    expect(listeners.size).toBe(0)
  })

  it("rejects with the helper's code, the decoder's without its `import.` prefix", async () => {
    for (const [code, expected] of [
      ['import.too_large', 'too_large'],
      ['import.corrupt', 'corrupt'],
      ['import.path', 'path'],
      ['import.no_folder', 'no_folder'],
      ['import.folder_changed', 'folder_changed'],
      ['import.cancelled', 'cancelled'],
      ['import.something_new', 'io'],
    ] as const) {
      const { p, started, say } = port()
      const running = runHelperImport(p, { kind: 'picked', token: '0'.repeat(32) }, () => {}, new AbortController().signal)
      say({ type: 'import.failed', id: started[0]!.id, code, message: 'm' })
      await expect(running, code).rejects.toMatchObject({ code: expected })
      expect(failureCode(code)).toBe(expected)
    }
  })

  it('a stop asks the helper to cancel and still waits for its last word: a blob put in place just before is known', async () => {
    const { p, started, cancelled, say } = port()
    const stop = new AbortController()
    const running = runHelperImport(p, { kind: 'folder', path: 'a.wav' }, () => {}, stop.signal)
    const id = started[0]!.id
    stop.abort()
    expect(cancelled).toEqual([id])
    let settled = false
    void running.then(
      () => (settled = true),
      () => (settled = true),
    )
    await Promise.resolve()
    expect(settled).toBe(false)
    say(done(id))
    await expect(running).resolves.toMatchObject({ created: true })
    // A stop before the start: the cancel follows the job at once.
    const again = port()
    const before = new AbortController()
    before.abort()
    const late = runHelperImport(again.p, { kind: 'folder', path: 'a.wav' }, () => {}, before.signal)
    expect(again.cancelled).toEqual([again.started[0]!.id])
    again.say({ type: 'import.failed', id: again.started[0]!.id, code: 'import.cancelled', message: '' })
    await expect(late).rejects.toMatchObject({ code: 'cancelled' })
  })

  it('the session ending, or the helper silent for its whole silence time, ends it as the helper lost', async () => {
    const { p, started, say } = port()
    const running = runHelperImport(p, { kind: 'folder', path: 'a.wav' }, () => {}, new AbortController().signal)
    say(null)
    await expect(running).rejects.toMatchObject({ code: 'helper_lost' })
    expect(started).toHaveLength(1)

    const silent = port()
    const clock = manualTimers()
    const watched = runHelperImport(silent.p, { kind: 'folder', path: 'a.wav' }, () => {}, new AbortController().signal, clock.timers)
    expect(clock.armed()).toBe(true)
    // Each word of the helper starts the wait again.
    silent.say({ type: 'import.progress', id: silent.started[0]!.id, fraction: 0.1 })
    expect(clock.armed()).toBe(true)
    clock.fire()
    await expect(watched).rejects.toBeInstanceOf(ImportError)
    await expect(watched).rejects.toMatchObject({ code: 'helper_lost' })
    expect(silent.cancelled).toEqual([silent.started[0]!.id])
    expect(HELPER_IMPORT_SILENCE_MS).toBeGreaterThanOrEqual(10_000)
  })

  it('a word after the end changes nothing', async () => {
    const { p, started, say } = port()
    const onProgress = vi.fn()
    const running = runHelperImport(p, { kind: 'folder', path: 'a.wav' }, onProgress, new AbortController().signal)
    const id = started[0]!.id
    say(done(id))
    await running
    say({ type: 'import.progress', id, fraction: 0.5 })
    say({ type: 'import.failed', id, code: 'import.write', message: '' })
    expect(onProgress).not.toHaveBeenCalled()
  })
})

describe("the helper's file dialog", () => {
  it('gives the files the user picked, none when the dialog was closed, and the refusal by its code', async () => {
    const { p, chosen, say } = port()
    const picking = chooseWithHelper(p)
    say({ type: 'import.chosen', id: chosen[0]! + 1, files: [] })
    say({ type: 'import.chosen', id: chosen[0]!, files: [{ token: 'a'.repeat(32), name: 'kick.wav' }] })
    expect(await picking).toEqual([{ token: 'a'.repeat(32), name: 'kick.wav' }])
    const closed = chooseWithHelper(p)
    say({ type: 'import.chosen', id: chosen[1]!, files: [] })
    expect(await closed).toEqual([])
    const refused = chooseWithHelper(p)
    say({ type: 'import.failed', id: chosen[2]!, code: 'import.dialog_unavailable', message: '' })
    await expect(refused).rejects.toMatchObject({ code: 'dialog_unavailable' })
    const lost = chooseWithHelper(p)
    say(null)
    await expect(lost).rejects.toMatchObject({ code: 'helper_lost' })
  })
})
