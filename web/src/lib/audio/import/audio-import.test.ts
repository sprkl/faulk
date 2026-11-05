// The page's audio import (AU-1) against stand-ins: the dialog only from a click, each file
// imported then staged then made a clip by `clip.import_audio` (in that order), a refused file
// reported while the next ones are still imported, a cancel, the progress.
import { describe, expect, it } from 'vitest'
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
