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
      // The instrument view holds no pressed button of its own, so the tab is the only one.
      const stores = createStores('arrangement', DEFAULT_PANEL)
      stores.ui.showPanel(view.id)
      const { body } = render(Shell, { props: { stores } })
      expect(body).toContain(`data-panel="${view.id}"`)
      expect(body).toMatch(new RegExp(`aria-pressed="true"[^>]*data-testid="panel-${view.id}"`))
      expect(body.match(/aria-pressed="true"[^>]*data-testid="panel-/g), view.id).toHaveLength(1)
    }
  })

  it('shows a pairing result in every view', () => {
    for (const view of viewsIn('main')) {
      const stores = createStores(view.id, DEFAULT_PANEL)
      stores.engine.pairingMessage = { key: 'helper.failed' }
      expect(render(Shell, { props: { stores } }).body, view.id).toContain('data-testid="pairing-notice"')
    }
  })

  it('says a pairing link waits for the audio, until the audio starts', () => {
    const stores = createStores('mixer', DEFAULT_PANEL)
    stores.engine.pendingPairCode = '12345678'
    expect(render(Shell, { props: { stores } }).body).toContain('data-testid="pair-pending-notice"')
    stores.engine.started = true
    expect(render(Shell, { props: { stores } }).body).not.toContain('data-testid="pair-pending-notice"')
  })

  it('has no Instrument view any more: the engine pill opens the dialog, and the keyboard is a panel tab', () => {
    const { body } = render(Shell, { props: { stores: createStores('arrangement', DEFAULT_PANEL) } })
    expect(body).not.toContain('data-testid="nav-play"')
    expect(body).toContain('data-testid="panel-keyboard"')
    expect(body).toContain('data-testid="engine-dialog"')
  })

  it('shows the engine pill as a button with the mode, the latency and the load and its peak when the engine measures them', () => {
    const stores = createStores('arrangement', DEFAULT_PANEL)
    stores.engine.started = true
    let pill = render(Shell, { props: { stores } }).body
    expect(pill).toMatch(/<button[^>]*aria-haspopup="dialog"[^>]*data-testid="engine-mode"/)
    expect(pill).not.toContain('data-testid="engine-load"')
    expect(pill).toContain('Not measured by this engine yet')
    stores.engine.load = { smoothed: 0.42, peak: 0.6 }
    pill = render(Shell, { props: { stores } }).body
    expect(pill).toContain('data-testid="engine-load"')
    expect(pill).toContain('CPU 42 %')
    expect(pill).toContain('CPU load 42 %, peak 60 %')
    expect(pill).toContain('42 % (peak 60 % over the last 3 s)')
    stores.ui.lang = 'fr'
    pill = render(Shell, { props: { stores } }).body
    expect(pill).toContain('Charge CPU 42 %, crête 60 %')
    expect(pill).toContain('42 % (crête 60 % sur les 3 dernières s)')
  })

  it('marks the engine pill once the output protection silenced a non-finite signal, and the dialog says how much (#667)', () => {
    const stores = createStores('arrangement', DEFAULT_PANEL)
    stores.engine.started = true
    stores.engine.status = { kind: 'web', sampleRate: 48_000, estimatedLatencyMs: null }
    let body = render(Shell, { props: { stores } }).body
    expect(body).not.toContain('data-testid="engine-silenced"')
    expect(body).not.toContain('Invalid signal silenced')
    expect(body).toMatch(/data-testid="engine-dialog-silenced"[^>]*>Nothing silenced</)
    stores.engine.silencedFrames = 4_800
    body = render(Shell, { props: { stores } }).body
    expect(body).toContain('data-testid="engine-silenced"')
    expect(body).toMatch(/aria-label="[^"]*Invalid signal silenced to protect your ears\. Audio engine and MIDI"/)
    expect(body).toMatch(/title="Audio engine and MIDI: [^"]*\. Invalid signal silenced to protect your ears"/)
    expect(body).toContain('100 ms of invalid signal silenced since the audio started')
    stores.engine.silencedFrames = 3
    stores.ui.lang = 'fr'
    body = render(Shell, { props: { stores } }).body
    expect(body).toContain('Signal invalide coupé pour protéger tes oreilles')
    expect(body).toContain('moins d’1 ms de signal invalide coupé depuis le démarrage de l’audio')
  })

  it('says in the engine dialog how much latency the devices add, which the engine compensates (#749)', () => {
    const stores = createStores('arrangement', DEFAULT_PANEL)
    stores.engine.started = true
    stores.engine.status = { kind: 'web', sampleRate: 48_000, estimatedLatencyMs: null }
    let body = render(Shell, { props: { stores } }).body
    expect(body).toMatch(/data-testid="engine-dialog-compensation"[^>]*>No device adds latency</)
    stores.engine.latencyFrames = 72
    body = render(Shell, { props: { stores } }).body
    expect(body).toContain('Devices add 1.5 ms (72 samples), compensated: everything stays in time')
    // Before the engine's rate is known (a helper session opening): the samples alone, never "none".
    stores.engine.status = null
    body = render(Shell, { props: { stores } }).body
    expect(body).toContain('Devices add 72 samples, compensated: everything stays in time')
    stores.engine.status = { kind: 'web', sampleRate: 48_000, estimatedLatencyMs: null }
    stores.ui.lang = 'fr'
    body = render(Shell, { props: { stores } }).body
    expect(body).toContain('Les appareils ajoutent 1,5 ms (72 échantillons), compensés : tout reste en place')
  })

  it('folds the bottom panel in the mixer and gives it back on leaving, without touching the stored fold', () => {
    const stores = createStores('arrangement', DEFAULT_PANEL)
    const ui = stores.ui
    expect(ui.panelFolded).toBe(false)
    ui.show('mixer')
    expect(ui.panelFolded).toBe(true)
    expect(ui.panelCollapsed).toBe(false)
    ui.showPanel('devices') // an action that targets the panel unfolds it for the visit
    expect(ui.panelFolded).toBe(false)
    ui.toggleFold()
    expect(ui.panelFolded).toBe(true)
    ui.show('arrangement')
    expect(ui.panelFolded).toBe(false)
    ui.setPanelCollapsed(true)
    ui.show('mixer')
    ui.unfoldPanel() // the user had folded it: unfolding is theirs to ask for
    expect(ui.panelFolded).toBe(false)
    expect(ui.panelCollapsed).toBe(false)
    ui.setPanelCollapsed(false)
  })

  it('maximises the panel, and a resize or a fold ends it', () => {
    const ui = createStores('arrangement', DEFAULT_PANEL).ui
    ui.toggleMaximized()
    expect(ui.panelMaximized).toBe(true)
    ui.toggleMaximized()
    expect(ui.panelMaximized).toBe(false)
    ui.toggleMaximized()
    ui.setPanelHeight('devices', 200)
    expect(ui.panelMaximized).toBe(false)
    ui.toggleMaximized()
    ui.toggleFold()
    expect(ui.panelFolded).toBe(true)
    ui.toggleFold()
  })

  it('shows the validated message when the engine stopped taking changes, in both languages', () => {
    const stores = createStores(DEFAULT_VIEW, DEFAULT_PANEL)
    expect(render(Shell, { props: { stores } }).body).not.toContain('data-testid="stream-error"')
    stores.engine.streamError = { code: -3, at: 0 }
    stores.ui.setLang('en')
    expect(render(Shell, { props: { stores } }).body).toContain('The audio engine stopped taking changes. Click "Start audio" to continue.')
    stores.ui.setLang('fr')
    expect(render(Shell, { props: { stores } }).body).toContain(
      'Le moteur audio n’accepte plus les changements. Clique sur « Démarrer l’audio » pour continuer.',
    )
  })
})
