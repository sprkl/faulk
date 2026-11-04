import { render } from 'svelte/server'
import { describe, expect, it } from 'vitest'
import { LIMITER_PUSHED_DB, type ExportResult } from '../../lib/export/audio-export.svelte'
import { createStores, storesContext, type Stores } from '../../lib/stores'
import { DEFAULT_PANEL, DEFAULT_VIEW } from '../registry'

// The export dialog's "pushed into the limiter" warning (#402): it is shown exactly when the
// reported gain reduction reaches LIMITER_PUSHED_DB. The condition lives in this component's
// template, so it is tested here, not only in the controller's dB conversion.

const { default: ExportDialog } = await import('./ExportDialog.svelte')

const result = (reductionDb: number): ExportResult => ({
  name: 'Song.wav',
  kind: 'file',
  bytes: 1_000,
  seconds: 2,
  peakDb: -0.3,
  clipped: 0,
  reductionDb,
  silencedFrames: 0,
  tailCut: false,
  missing: [],
  renderMs: 10,
})

/** Stores with the export dialog open on a finished export of the given gain reduction. */
function doneWith(reductionDb: number): Stores {
  const stores = createStores(DEFAULT_VIEW, DEFAULT_PANEL)
  stores.audioExport.open = true
  stores.audioExport.phase = 'done'
  stores.audioExport.result = result(reductionDb)
  return stores
}

const body = (reductionDb: number) => render(ExportDialog, { context: storesContext(doneWith(reductionDb)) }).body

describe('the export dialog’s limiter warning', () => {
  it('shows no warning when the limiter barely engaged', () => {
    const b = body(LIMITER_PUSHED_DB - 0.1)
    expect(b).toContain('data-testid="export-peak"')
    expect(b).not.toContain('data-testid="export-limited"')
  })

  it('shows the warning at and above the threshold, with the gain reduction', () => {
    const at = body(LIMITER_PUSHED_DB)
    expect(at).toContain('data-testid="export-limited"')
    const over = body(LIMITER_PUSHED_DB + 6)
    expect(over).toContain('data-testid="export-limited"')
    // The warning names the gain reduction it reports.
    expect(over).toContain((LIMITER_PUSHED_DB + 6).toFixed(1))
  })

  it('shows no warning for a mix that never engaged the limiter (0 dB)', () => {
    expect(body(0)).not.toContain('data-testid="export-limited"')
  })
})

describe('the export dialog’s unread audio files (#647)', () => {
  it('names the files whose clips are silent, and says nothing when every file was read', () => {
    const stores = doneWith(0)
    stores.audioExport.result = { ...result(0), missing: ['kick.wav', 'voice.mp3'] }
    const b = render(ExportDialog, { context: storesContext(stores) }).body
    expect(b).toContain('data-testid="export-missing"')
    expect(b).toContain('kick.wav, voice.mp3')
    expect(body(0)).not.toContain('data-testid="export-missing"')
  })
})
