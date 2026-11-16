import { render } from 'svelte/server'
import { describe, expect, it } from 'vitest'
import { createStores, storesContext } from '../../lib/stores'
import BottomPanel from './BottomPanel.svelte'

type Stores = ReturnType<typeof createStores>

function panel(setup: (stores: Stores) => void, lang: 'en' | 'fr' = 'en') {
  const stores = createStores('arrangement', 'clips')
  stores.ui.lang = lang
  setup(stores)
  return render(BottomPanel, { context: storesContext(stores), props: { max: 600 } }).body
}

describe('the step entry badge of the panel (#916)', () => {
  it('shows when step entry is armed and no clip is written into, in both languages', () => {
    const armed = (stores: Stores) => {
      stores.ui.stepEntry = true
      stores.engine.started = true
    }
    expect(panel(armed)).toMatch(/data-testid="panel-step-armed"[^>]*>Step entry on: letters play</)
    expect(panel(armed, 'fr')).toContain('Saisie pas à pas : les lettres jouent')
  })

  it('says when the audio is not started, so the letters play nothing', () => {
    expect(panel((stores) => (stores.ui.stepEntry = true))).toContain('Step entry on: audio off')
  })

  it('is absent when step entry is off or the roll writes', () => {
    expect(panel(() => {})).not.toContain('panel-step-armed')
    expect(
      panel((stores) => {
        stores.ui.stepEntry = true
        stores.ui.stepWriting = true
      }),
    ).not.toContain('panel-step-armed')
  })
})
