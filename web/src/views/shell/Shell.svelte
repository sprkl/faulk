<script lang="ts">
  /**
   * The DAW's layout: top bar (views, transport, engine state), track list, main area showing the
   * registry's current view, bottom panel (devices, piano roll). It creates the stores once and
   * provides them to every view.
   */
  import { goto } from '$app/navigation'
  import { onMount } from 'svelte'
  import { watchLocalAccess } from '../../lib/engine/local-access'
  import { historyShortcut, saveShortcut } from '../../lib/keyboard'
  import { save as saveCommand, saveAs as saveAsCommand } from '../../lib/project-commands'
  import { redo, undo } from '../../lib/track-commands'
  import { createStores, provideStores, type Stores } from '../../lib/stores'
  import { DEFAULT_PANEL, DEFAULT_VIEW, viewById } from '../registry'
  import BottomPanel from './BottomPanel.svelte'
  import { clampHeight, panelMax } from './panel-size'
  import ExportDialog from './ExportDialog.svelte'
  import StorageBanners from './StorageBanners.svelte'
  import TopBar from './TopBar.svelte'
  import TrackList from '../tracks/TrackList.svelte'

  // `stores` is for tests; the page lets the shell create its own.
  let { stores = createStores(DEFAULT_VIEW, DEFAULT_PANEL) }: { stores?: Stores } = $props()
  // svelte-ignore state_referenced_locally
  const { engine, ui, dispatcher, storage, keys, meters } = provideStores(stores)

  const current = $derived(viewById(ui.view) ?? viewById(DEFAULT_VIEW)!)

  // The panel's row: folded to its tabs, or the height the user gave this view; else the stylesheet's default.
  let bodyHeight = $state(0)
  const stored = $derived(ui.panelHeights[ui.panel])
  const panelRow = $derived(
    ui.panelCollapsed ? 'auto' : stored === undefined ? undefined : `${bodyHeight > 0 ? clampHeight(ui.panel, stored, bodyHeight) : stored}px`,
  )

  onMount(() => {
    const stopMeters = meters.start()
    void dispatcher.checkHelper()
    // The project of last time comes back (or a banner offers it).
    void storage.restore()
    // The first probe may wait behind the browser's local network prompt: probe again on Allow.
    const stopLocalAccess = watchLocalAccess(
      () => void dispatcher.onLocalAccessGranted(),
      undefined,
      (state) => (engine.localAccess = state),
    )
    // Through the router: a raw history.replaceState conflicts with SvelteKit's.
    // `url` is the current path and query, already resolved against the base path.
    // eslint-disable-next-line svelte/no-navigation-without-resolve
    const clearAddress = (url: string) => void goto(url, { shallow: true, replace: true })
    dispatcher.takePairLink(location, clearAddress)
    const onHashChange = () => dispatcher.takePairLink(location, clearAddress)
    window.addEventListener('hashchange', onHashChange)
    return () => {
      stopMeters()
      stopLocalAccess()
      window.removeEventListener('hashchange', onHashChange)
    }
  })

  function onKeyDown(event: KeyboardEvent) {
    const save = saveShortcut(event)
    if (save) {
      // The browser would offer to save the page as a file.
      event.preventDefault()
      dispatcher.run(save === 'save' ? saveCommand() : saveAsCommand())
      return
    }
    const action = historyShortcut(event)
    if (!action) {
      // The computer keyboard plays in every view; what is no shortcut of the shell goes to the notes.
      keys.keyDown(event)
      return
    }
    event.preventDefault()
    // Undo pressed during a fader drag (a history group is open) does nothing, and says nothing.
    dispatcher.run(action === 'undo' ? undo() : redo(), { quiet: true })
  }

  onMount(() => {
    const release = () => keys.releaseAll()
    const onHidden = () => document.hidden && keys.releaseAll()
    window.addEventListener('blur', release)
    document.addEventListener('visibilitychange', onHidden)
    return () => {
      window.removeEventListener('blur', release)
      document.removeEventListener('visibilitychange', onHidden)
      keys.releaseAll()
    }
  })

  $effect(() => {
    document.documentElement.lang = ui.lang
  })

  /** Leaving with changes no folder holds loses them: the browser asks first (its own words). */
  function onBeforeUnload(event: BeforeUnloadEvent) {
    if (!storage.unsaved) return
    event.preventDefault()
    event.returnValue = ''
  }

  /** In the background a tab may be frozen or closed without notice: write what waits; coming back, look at the folder again. */
  function onVisibility() {
    if (document.visibilityState === 'hidden') storage.sleep()
    else void storage.wake()
  }
</script>

<svelte:window onkeydown={onKeyDown} onkeyup={(event) => keys.keyUp(event)} onbeforeunload={onBeforeUnload} onfocus={() => void storage.wake()} />
<svelte:document onvisibilitychange={onVisibility} />

<div class="shell">
  <TopBar />
  <StorageBanners />
  <ExportDialog />
  {#if engine.notice}
    <p class="notice" role="status">{ui.t(engine.notice)}</p>
  {/if}
  {#if engine.streamError}
    <p class="notice" role="alert" data-testid="stream-error">{ui.t('engine.streamError')}</p>
  {/if}
  <!-- The instrument view shows pairing state in its own panels; elsewhere, here. -->
  {#if engine.pendingPairCode && !engine.started && ui.view !== 'play'}
    <p class="notice" role="status" data-testid="pair-pending-notice">{ui.t('start.pairPending')}</p>
  {/if}
  {#if engine.pairingMessage && ui.view !== 'play'}
    <p class="notice" role="status" data-testid="pairing-notice">{ui.t(engine.pairingMessage.key, { n: engine.pairingMessage.n ?? 0 })}</p>
  {/if}
  {#if engine.refusal}
    {@const text = ui.t(engine.refusal.key, { ...engine.refusal.vars, what: engine.refusal.whatKey ? ui.t(engine.refusal.whatKey) : '' })}
    <p class="toast" role="status" data-testid="refusal-toast">{engine.refusal.count > 1 ? ui.t('refusal.count', { text, n: engine.refusal.count }) : text}</p>
  {/if}
  <div class="body" data-panel={ui.panel} bind:clientHeight={bodyHeight} style:--panel-row={panelRow}>
    <TrackList />
    <main class="main" aria-label={ui.t(current.label)} data-testid="view" data-view={current.id}>
      <current.component />
    </main>
    <BottomPanel max={panelMax(ui.panel, bodyHeight)} />
  </div>
</div>

<style>
  .shell {
    height: 100vh;
    height: 100dvh;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .notice {
    flex-shrink: 0;
    margin: 0;
    padding: var(--fk-space-2) var(--fk-space-4);
    border-bottom: 1px solid var(--fk-line);
    background: var(--fk-surface);
  }
  /* A refusal of the core: temporary, over the page, never in the way of a click. */
  .toast {
    position: fixed;
    left: 50%;
    bottom: var(--fk-space-4);
    transform: translateX(-50%);
    z-index: 50;
    max-width: min(32rem, calc(100vw - 2 * var(--fk-space-4)));
    margin: 0;
    padding: var(--fk-space-2) var(--fk-space-4);
    border: 1px solid var(--fk-line);
    border-radius: var(--fk-radius-panel);
    background: var(--fk-surface);
    box-shadow: var(--fk-shadow-float);
    pointer-events: none;
  }
  .body {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: 272px minmax(0, 1fr);
    grid-template-rows: minmax(0, 1fr) var(--panel-row, minmax(120px, 28%));
    grid-template-areas:
      'tracks main'
      'tracks panel';
  }
  /* The piano roll needs rows to draw in: its panel takes more of the height than the device panel. */
  .body[data-panel='piano-roll'] {
    /* In fr, not %: a percentage track resolves to its content height here and pushes the roll's lane off the screen. */
    grid-template-rows: minmax(0, 1fr) var(--panel-row, minmax(360px, 1.2fr));
  }
  .main {
    grid-area: main;
    min-width: 0;
    overflow: auto;
  }
  /* Narrow screens: one column that scrolls as a whole. */
  @media (max-width: 760px) {
    .shell {
      height: auto;
      min-height: 100vh;
      overflow: visible;
    }
    .body {
      grid-template-columns: minmax(0, 1fr);
      grid-template-rows: auto auto auto;
      grid-template-areas: 'main' 'panel' 'tracks';
    }
    .main {
      overflow: visible;
    }
  }
</style>
