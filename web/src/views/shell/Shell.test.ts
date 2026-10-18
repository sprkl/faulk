import { render } from 'svelte/server'
import { describe, expect, it, vi } from 'vitest'
import { createStores } from '../../lib/stores'
import { DEFAULT_PANEL, DEFAULT_VIEW, viewsIn } from '../registry'

// No audio and no router here: the shell is rendered without starting an engine. Clicking through
// the views runs in a real browser (Playwright); here, what the shell renders for each state.
vi.mock('$app/navigation', () => ({ goto: vi.fn() }))
vi.mock('$app/paths', () => ({ resolve: (path: string) => path }))
vi.mock('../../lib/engine/web-engine', () => ({ WebEngine: { create: () => Promise.reject(new Error('no audio')) } }))
vi.mock('../../lib/engine/native-engine', () => ({ NativeEngine: { connect: () => Promise.reject(new Error('no helper')) } }))

const { default: Shell } = await import('./Shell.svelte')

const ids = (body: string, prefix: string) => [...body.matchAll(new RegExp(`data-testid="${prefix}-([a-z-]+)"`, 'g'))].map((m) => m[1])

describe('the shell', () => {
  it('lays out the top bar, the track list, the main area and the bottom panel', () => {
    const { body } = render(Shell)
    for (const id of ['engine-state', 'track-list', 'view', 'bottom-panel']) expect(body, id).toContain(`data-testid="${id}"`)
  })

  it('says the project loads with the audio, and offers no track before', () => {
    const { body } = render(Shell)
    expect(body).toContain('data-testid="tracks-waiting"')
    expect(body).not.toContain('data-testid="add-track"')
    expect(body).not.toContain('data-testid="track-header"')
  })

  it('lists the main views in the top bar and the panel views in the bottom panel, in registry order', () => {
    const { body } = render(Shell)
    expect(ids(body, 'nav')).toEqual(viewsIn('main').map((view) => view.id))
    expect(ids(body, 'panel')).toEqual(viewsIn('panel').map((view) => view.id))
    expect(body).toContain(`data-view="${DEFAULT_VIEW}"`)
    expect(body).toContain(`data-panel="${DEFAULT_PANEL}"`)
  })

  it('shows the chosen main view and marks only its button as current', () => {
    for (const view of viewsIn('main')) {
      const stores = createStores(DEFAULT_VIEW, DEFAULT_PANEL)
      stores.ui.show(view.id)
      const { body } = render(Shell, { props: { stores } })
      expect(body).toContain(`data-view="${view.id}"`)
      expect(body.match(/aria-current="page"/g)).toHaveLength(1)
      expect(body).toMatch(new RegExp(`aria-current="page"[^>]*data-testid="nav-${view.id}"`))
    }
  })

  it('shows the chosen panel view and marks only its tab as pressed', () => {
    for (const view of viewsIn('panel')) {
      const stores = createStores(DEFAULT_VIEW, DEFAULT_PANEL)
      stores.ui.showPanel(view.id)
      const { body } = render(Shell, { props: { stores } })
      expect(body).toContain(`data-panel="${view.id}"`)
      expect(body).toMatch(new RegExp(`aria-pressed="true"[^>]*data-testid="panel-${view.id}"`))
      expect(body.match(/aria-pressed="true"/g)).toHaveLength(1)
    }
  })

  it('shows a pairing result outside the instrument view only', () => {
    const stores = createStores('grid', DEFAULT_PANEL)
    stores.engine.pairingMessage = { key: 'helper.failed' }
    expect(render(Shell, { props: { stores } }).body).toContain('data-testid="pairing-notice"')
    stores.ui.show('play')
    expect(render(Shell, { props: { stores } }).body).not.toContain('data-testid="pairing-notice"')
  })

  it('says a pairing link waits for the audio outside the instrument view only', () => {
    const stores = createStores('mixer', DEFAULT_PANEL)
    stores.engine.pendingPairCode = '12345678'
    expect(render(Shell, { props: { stores } }).body).toContain('data-testid="pair-pending-notice"')
    stores.ui.show('play')
    expect(render(Shell, { props: { stores } }).body).not.toContain('data-testid="pair-pending-notice"')
    stores.ui.show('mixer')
    stores.engine.started = true
    expect(render(Shell, { props: { stores } }).body).not.toContain('data-testid="pair-pending-notice"')
  })
  it('shows the validated message when the engine stopped taking changes, in both languages', () => {
    const stores = createStores(DEFAULT_VIEW, DEFAULT_PANEL)
    expect(render(Shell, { props: { stores } }).body).not.toContain('data-testid="stream-error"')
    stores.engine.streamError = { code: -3, at: 0 }
    stores.ui.setLang('en')
    expect(render(Shell, { props: { stores } }).body).toContain('The audio engine stopped taking changes. Click "Start audio" to continue.')
    stores.ui.setLang('fr')
    expect(render(Shell, { props: { stores } }).body).toContain('Le moteur audio n’accepte plus les changements. Clique sur « Démarrer l’audio » pour continuer.')
  })
})
