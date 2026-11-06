// The page's audio import (AU-1) against stand-ins: the dialog only from a click, each file
// imported then staged then made a clip by `clip.import_audio` (in that order), a refused file
// reported while the next ones are still imported, a cancel, the progress.
import { describe, expect, it, vi } from 'vitest'
import type { AudioMedia, Command } from '../../generated/protocol'
import { StoreError } from '../../storage/types'
import { ImportError } from './abi'
import { AudioImport, importCommand, type AudioImportDeps } from './audio-import.svelte'
import type { ImportedMedia } from './pipeline'

const media = (name: string, i: number): ImportedMedia => ({ hash: i.toString(16).repeat(64), sample_rate: 44_100, channels: 2, frames: 22_050, name })

function setup(over: Partial<AudioImportDeps> = {}) {
  const log: string[] = []
  const staged: AudioMedia[] = []
  const ran: Command[] = []
  let picked: File[] = []
  const deps: AudioImportDeps = {
    canAsk: () => true,
    pick: async () => {
      log.push('pick')
      return picked
    },
    importFile: async (file, onProgress) => {
      log.push(`import ${file.name}`)
      onProgress(0.5)
      return media(file.name, log.length)
    },
    stage: (m) => {
      log.push(`stage ${m.name}`)
      staged.push(m)
      return true
    },
    run: (command) => {
      log.push(`run ${command.type}`)
      ran.push(command)
      return true
    },
    pageHostname: () => 'faulk.app',
    download: async (_url, name) => ({ status: 'ok', file: new File([new Uint8Array(4)], name) }),
    ...over,
  }
  return { importer: new AudioImport(deps), log, staged, ran, pick: (files: File[]) => (picked = files) }
}

const file = (name: string) => new File([new Uint8Array(4)], name)

describe('importing audio files from the page', () => {
  it('opens the dialog from a click, then imports, stages and makes a clip of each file in turn', async () => {
    const { importer, log, ran, pick } = setup()
    pick([file('kick.wav'), file('snare.wav')])
    importer.choose('T1')
    await importer.importFiles('T1', [])
    expect(log).toEqual(['pick', 'import kick.wav', 'stage kick.wav', 'run clip.import_audio', 'import snare.wav', 'stage snare.wav', 'run clip.import_audio'])
    expect(ran[0]).toEqual(importCommand('T1', media('kick.wav', 2).hash))
    expect(importer.running).toBeNull()
    expect(importer.failure).toBeNull()
  })

  it('hands each staged file to a placer when one is given (AU-6), and reports one that makes no clip', async () => {
    const { importer, log, ran } = setup()
    const placed: [string, number][] = []
    const placer = (m: AudioMedia, index: number) => {
      placed.push([m.name, index])
      return index === 0
    }
    await importer.importFiles('T1', [file('kick.wav'), file('snare.wav')], placer)
    // The placer runs its own commands: none goes through the default import.
    expect(ran).toEqual([])
    expect(log).toEqual(['import kick.wav', 'stage kick.wav', 'import snare.wav', 'stage snare.wav'])
    expect(placed).toEqual([
      ['kick.wav', 0],
      ['snare.wav', 1],
    ])
    expect(importer.failure).toMatchObject({ name: 'snare.wav', code: 'store' })
  })

  it('never opens a dialog without a click: it says so instead', () => {
    const { importer, log } = setup({ canAsk: () => false })
    importer.choose('T1')
    expect(log).toEqual([])
    expect(importer.needsClick).toBe(true)
    importer.dismiss()
    expect(importer.needsClick).toBe(false)
  })

  it('reports a refused file and still imports the next ones; the folder refusing a write is reported too', async () => {
    let n = 0
    const { importer, ran } = setup({
      importFile: async (f) => {
        n++
        if (f.name === 'song.ac3') throw new ImportError('unsupported_codec', 'AC-3')
        if (f.name === 'big.wav') throw new StoreError('full', 'quota')
        return media(f.name, n)
      },
    })
    await importer.importFiles('T1', [file('song.ac3'), file('kick.wav')])
    expect(importer.failure).toMatchObject({ name: 'song.ac3', code: 'unsupported_codec', track: 'T1' })
    expect(ran.length).toBe(1)
    await importer.importFiles('T1', [file('big.wav')])
    expect(importer.failure).toMatchObject({ name: 'big.wav', code: 'full' })
    // A new import starts clean.
    await importer.importFiles('T1', [file('ok.wav')])
    expect(importer.failure).toBeNull()
  })

  it('a project not ready, or a clip the core refused, is a failure, not a silent loss', async () => {
    const noCore = setup({ stage: () => false })
    await noCore.importer.importFiles('T1', [file('a.wav')])
    expect(noCore.importer.failure?.code).toBe('unavailable')
    expect(noCore.ran).toEqual([])
    const refused = setup({ run: () => false })
    await refused.importer.importFiles('T1', [file('a.wav')])
    expect(refused.importer.failure?.code).toBe('store')
  })

  it('a cancel stops the file under way and the ones after it, and says nothing went wrong', async () => {
    const seen: string[] = []
    const { importer, ran } = setup({
      importFile: (f, _progress, signal) =>
        new Promise((resolve, reject) => {
          seen.push(f.name)
          signal.addEventListener('abort', () => reject(new ImportError('cancelled', 'cancelled')))
          if (f.name === 'fast.wav') resolve(media(f.name, 1))
        }),
    })
    const running = importer.importFiles('T1', [file('slow.wav'), file('next.wav')])
    await Promise.resolve()
    expect(importer.running).toMatchObject({ name: 'slow.wav', index: 1, count: 2 })
    importer.cancel()
    await running
    expect(seen).toEqual(['slow.wav'])
    expect(ran).toEqual([])
    expect(importer.failure).toBeNull()
    expect(importer.running).toBeNull()
  })

  it('shows how far the file under way is', async () => {
    let report: (f: number) => void = () => {}
    let finish: () => void = () => {}
    const { importer } = setup({
      importFile: (f, onProgress) =>
        new Promise((resolve) => {
          report = onProgress
          finish = () => resolve(media(f.name, 1))
        }),
    })
    const running = importer.importFiles('T1', [file('long.wav')])
    await Promise.resolve()
    report(0.25)
    expect(importer.running?.fraction).toBe(0.25)
    report(0.75)
    expect(importer.running?.fraction).toBe(0.75)
    finish()
    await running
    expect(importer.running).toBeNull()
  })
})

describe("an agent's file from a link (#664)", () => {
  const LINK = new URL('https://example.org/kick.wav')
  const placer = (made: AudioMedia[]) => (m: AudioMedia) => {
    made.push(m)
    return true
  }

  it('downloads the file, then imports it as a picked file and has the placer make its clip', async () => {
    const made: AudioMedia[] = []
    const { importer, log } = setup({
      download: async (url, name, onProgress) => {
        log.push(`download ${url.href} as ${name}`)
        onProgress(2, 4)
        expect(importer.downloads).toEqual([{ track: 'T1', name, host: 'example.org', received: 2, total: 4 }])
        return { status: 'ok', file: new File([new Uint8Array(4)], name) }
      },
    })
    const got = await importer.importUrl('T1', LINK, 'example.org', 'kick.wav', placer(made))
    expect(got).toEqual({ status: 'ok', hash: made[0]!.hash })
    expect(log).toEqual(['download https://example.org/kick.wav as kick.wav', 'import kick.wav', 'stage kick.wav'])
    expect(importer.downloads).toEqual([])
    expect(importer.running).toBeNull()
    expect(importer.importing(LINK)).toBe(false)
  })

  it('checks the link with the page’s own host: plain http only to this computer from a page served by it', () => {
    const deployed = setup({ pageHostname: () => 'faulk.app' })
    expect(deployed.importer.checkLink('http://localhost:5491/kick.wav').status).toBe('refused')
    expect(deployed.importer.checkLink('https://user:pw@example.org/kick.wav').status).toBe('refused')
    expect(deployed.importer.checkLink('https://example.org/kick.wav')).toMatchObject({ status: 'ok', host: 'example.org' })
    const local = setup({ pageHostname: () => 'localhost' })
    expect(local.importer.checkLink('http://localhost:5491/kick.wav').status).toBe('ok')
  })

  it('answers each way a download ends without a file with its own code, and shows it in the panel', async () => {
    for (const [answer, code, failure] of [
      [{ status: 'tooBig' }, 'download.too_big', 'download_too_big'],
      [{ status: 'unreachable', detail: 'HTTP 403' }, 'download.unreachable', 'download_unreachable'],
      [{ status: 'cancelled' }, 'import.cancelled', null],
    ] as const) {
      const made: AudioMedia[] = []
      const { importer, log } = setup({ download: async () => answer })
      const got = await importer.importUrl('T1', LINK, 'example.org', 'kick.wav', placer(made))
      expect(got).toMatchObject({ status: 'refused', code })
      expect(importer.failure?.code ?? null).toBe(failure)
      // Nothing reached the store or the project.
      expect(log).toEqual([])
      expect(made).toEqual([])
      expect(importer.importing(LINK)).toBe(false)
    }
  })

  it('a download that throws is a failed download, and the link is free again', async () => {
    const { importer, log } = setup({ download: () => Promise.reject(new RangeError('Array buffer allocation failed')) })
    const made: AudioMedia[] = []
    expect(await importer.importUrl('T1', LINK, 'example.org', 'kick.wav', placer(made))).toMatchObject({
      status: 'refused',
      code: 'download.unreachable',
      message: expect.stringContaining('Array buffer allocation failed'),
    })
    expect(log).toEqual([])
    expect(importer.downloads).toEqual([])
    expect(importer.importing(LINK)).toBe(false)
  })

  it('imports nothing when the project it was allowed for was replaced before its turn', async () => {
    let wanted = true
    const { importer, log } = setup({
      download: async (_url, name) => {
        wanted = false
        return { status: 'ok', file: new File([new Uint8Array(4)], name) }
      },
    })
    const made: AudioMedia[] = []
    const got = await importer.importUrl('T1', LINK, 'example.org', 'kick.wav', placer(made), undefined, () => wanted)
    expect(got).toMatchObject({ status: 'refused', code: 'import.project_changed' })
    expect(log).toEqual([])
    expect(made).toEqual([])
  })

  it('answers a file the decoder refuses with import.<its reason>, whatever the link called it', async () => {
    const { importer } = setup({
      importFile: async () => {
        throw new ImportError('unsupported_format', 'not audio')
      },
    })
    const made: AudioMedia[] = []
    const got = await importer.importUrl('T1', LINK, 'example.org', 'kick.wav', placer(made))
    expect(got).toEqual({ status: 'refused', code: 'import.unsupported_format', message: 'not audio: nothing changed' })
    expect(made).toEqual([])
    expect(importer.failure).toMatchObject({ name: 'kick.wav', code: 'unsupported_format' })
  })

  it('refuses a second import of a link under way, and takes it again once the first ended', async () => {
    let finish: () => void = () => {}
    const { importer } = setup({
      download: (_url, name) =>
        new Promise((resolve) => {
          finish = () => resolve({ status: 'ok', file: new File([new Uint8Array(4)], name) })
        }),
    })
    const made: AudioMedia[] = []
    const first = importer.importUrl('T1', LINK, 'example.org', 'kick.wav', placer(made))
    expect(importer.importing(LINK)).toBe(true)
    expect(await importer.importUrl('T1', LINK, 'example.org', 'kick.wav', placer(made))).toMatchObject({ status: 'refused', code: 'import.busy' })
    finish()
    expect(await first).toMatchObject({ status: 'ok' })
    expect(made).toHaveLength(1)
    const again = importer.importUrl('T1', LINK, 'example.org', 'kick.wav', placer(made))
    finish()
    expect(await again).toMatchObject({ status: 'ok' })
  })

  it('downloads at once while a batch of the user’s is imported, then imports after it', async () => {
    const order: string[] = []
    let finishBatch: () => void = () => {}
    const { importer } = setup({
      importFile: (f) => {
        order.push(`import ${f.name}`)
        return f.name === 'batch.wav' ? new Promise((resolve) => (finishBatch = () => resolve(media(f.name, 1)))) : Promise.resolve(media(f.name, 2))
      },
      download: async (_url, name) => {
        order.push('download')
        return { status: 'ok', file: new File([new Uint8Array(4)], name) }
      },
    })
    const batch = importer.importFiles('T1', [file('batch.wav')])
    await Promise.resolve()
    const made: AudioMedia[] = []
    const fromLink = importer.importUrl('T1', LINK, 'example.org', 'kick.wav', placer(made))
    await vi.waitFor(() => expect(order).toEqual(['import batch.wav', 'download']))
    finishBatch()
    await batch
    expect(await fromLink).toMatchObject({ status: 'ok' })
    expect(order).toEqual(['import batch.wav', 'download', 'import kick.wav'])
  })

  it('stops on a cancel (clip.cancel_audio_import) or when the agent’s call ends, during the download and before the import', async () => {
    for (const stopper of ['cancel', 'signal'] as const) {
      const call = new AbortController()
      const { importer, log } = setup({
        download: (_url, _name, _progress, signal) =>
          new Promise((resolve) => signal.addEventListener('abort', () => resolve({ status: 'cancelled' }), { once: true })),
      })
      const made: AudioMedia[] = []
      const running = importer.importUrl('T1', LINK, 'example.org', 'kick.wav', placer(made), call.signal)
      await Promise.resolve()
      if (stopper === 'cancel') importer.cancel()
      else call.abort()
      expect(await running, stopper).toMatchObject({ status: 'refused', code: 'import.cancelled' })
      expect(log).toEqual([])
      expect(made).toEqual([])
      expect(importer.downloads).toEqual([])
    }
    // Downloaded, then the call ends or a cancel comes while a batch of the user's holds the queue: the file is not imported.
    for (const stopper of ['cancel', 'signal'] as const) {
      let finishBatch: () => void = () => {}
      const call = new AbortController()
      const imported: string[] = []
      const { importer } = setup({
        importFile: (f) => {
          imported.push(f.name)
          return f.name === 'batch.wav' ? new Promise((resolve) => (finishBatch = () => resolve(media(f.name, 1)))) : Promise.resolve(media(f.name, 2))
        },
      })
      const batch = importer.importFiles('T1', [file('batch.wav')])
      await Promise.resolve()
      const made: AudioMedia[] = []
      const fromLink = importer.importUrl('T1', LINK, 'example.org', 'kick.wav', placer(made), call.signal)
      await vi.waitFor(() => expect(importer.downloads).toEqual([]))
      if (stopper === 'cancel') importer.cancel()
      else call.abort()
      finishBatch()
      await batch
      expect(await fromLink, stopper).toMatchObject({ status: 'refused', code: 'import.cancelled' })
      expect(imported, stopper).toEqual(['batch.wav'])
      expect(made, stopper).toEqual([])
      expect(importer.importing(LINK)).toBe(false)
    }
  })
})
