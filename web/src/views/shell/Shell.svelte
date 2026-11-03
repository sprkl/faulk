<script lang="ts">
  /**
   * The DAW's layout: top bar (views, transport, engine state), track list, main area showing the
   * registry's current view, bottom panel (devices, piano roll). It creates the stores once and
   * provides them to every view.
   */
  import { goto } from '$app/navigation'
  import { onMount } from 'svelte'
  import { watchLocalAccess } from '../../lib/engine/local-access'
  import { isMenuKey, menuPointOf, unhandledContextMenu } from '../../lib/context-menu'
  import { historyShortcut, isTextTarget, layoutShortcut, saveShortcut } from '../../lib/keyboard'
  import { takeOpenLink } from '../../lib/open-link'
  import { isPairHash } from '../../lib/pair-link'
  import { helpShortcut } from '../../lib/shortcuts'
  import { save as saveCommand, saveAs as saveAsCommand } from '../../lib/project-commands'
  import { runHistory } from '../../lib/history-said'
  import { createStores, provideStores, type Stores } from '../../lib/stores'
  import { DEFAULT_PANEL, DEFAULT_VIEW, startView, viewById, viewsIn, type MainViewId } from '../registry'
  import ContextMenu from '../../lib/components/ContextMenu.svelte'
  import BottomPanel from './BottomPanel.svelte'
  import { clampHeight, panelMax } from './panel-size'
  import EngineDialog from './EngineDialog.svelte'
  import ExportDialog from './ExportDialog.svelte'
  import ShortcutsDialog from './ShortcutsDialog.svelte'
  import StorageBanners from './StorageBanners.svelte'
  import TopBar from './TopBar.svelte'
  import TrackList from '../tracks/TrackList.svelte'

  // `stores` is for tests; the page lets the shell create its own.
  let { stores = createStores(startView(), DEFAULT_PANEL) }: { stores?: Stores } = $props()
  // svelte-ignore state_referenced_locally
  const { engine, ui, dispatcher, storage, keys, meters, project } = provideStores(stores)

  const current = $derived(viewById(ui.view) ?? viewById(DEFAULT_VIEW)!)

  // Each project comes back with its layout (#473): the view, the zoom and scroll, the open lanes, the lane heights.
  const isMainView = (id: string): id is MainViewId => viewById(id)?.place === 'main'
  $effect(() => ui.useProject(project.view?.project.id ?? null, isMainView))

  // The panel's row: folded to its tabs, or the height the user gave this view; else the stylesheet's default.
  let bodyHeight = $state(0)
  const stored = $derived(ui.panelHeights[ui.panel])
  const panelRow = $derived(
    ui.panelFolded
      ? 'auto'
      : ui.panelMaximized && bodyHeight > 0
        ? `${panelMax(ui.panel, bodyHeight)}px`
        : stored === undefined
          ? undefined
          : `${bodyHeight > 0 ? clampHeight(ui.panel, stored, bodyHeight) : stored}px`,
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
    // A project link (`?open=`, #497): asked about first, then opened by `project.open_url`. The address
    // is rewritten once, whether it held a project link, a pairing link or both.
    let withoutLink = null as string | null
    const link = takeOpenLink(location, (url) => (withoutLink = url))
    const address = withoutLink === null ? location : new URL(withoutLink, location.href)
    dispatcher.takePairLink(address, clearAddress)
    if (withoutLink !== null && !isPairHash(address.hash)) clearAddress(withoutLink)
    if (link !== null) storage.offerLink(link)
    const onHashChange = () => dispatcher.takePairLink(location, clearAddress)
    window.addEventListener('hashchange', onHashChange)
    // A reload or a closed tab keeps the layout of the moment.
    const keepLayout = () => ui.keepNow()
    window.addEventListener('pagehide', keepLayout)
    return () => {
      stopMeters()
      stopLocalAccess()
      window.removeEventListener('hashchange', onHashChange)
      window.removeEventListener('pagehide', keepLayout)
    }
  })

  // --- the right click (#524) ---------------------------------------------------------------------------------------
  // The browser's menu never opens in the app, except in a text field. A view whose target has actions opens the shared
  // menu from its own `contextmenu` handler (`ui.openMenu`); what no one handled ends here, cancelled, and opens nothing.
  let menuKeyAt = -Infinity

  /** Shift+F10 and the Menu key: the `contextmenu` event of the focused element, at its middle, as a right click would send it. */
  function askMenu(event: KeyboardEvent) {
    const target = (document.activeElement ?? document.body) as HTMLElement
    if (isTextTarget(target) && target !== document.body) return
    event.preventDefault()
    const box = target.getBoundingClientRect()
    const { x, y } = menuPointOf(box)
    menuKeyAt = performance.now()
    target.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, composed: true, clientX: x, clientY: y, button: 2, view: window }))
  }

  /** The browser sends its own `contextmenu` for the key too (on key up, on some systems): the one we sent already answered it. */
  function dropEcho(event: MouseEvent) {
    if (event.isTrusted && performance.now() - menuKeyAt < 600 && !isTextTarget(event.target)) {
      event.preventDefault()
      event.stopImmediatePropagation()
    }
  }

  function onContextMenu(event: MouseEvent) {
    // Nothing under the pointer has an action. An open menu closes on the press itself (ContextMenu), not here: the
    // browser's own echo of a Shift+F10 must not close the menu that key just opened.
    if (unhandledContextMenu(event) === 'cancel') event.preventDefault()
  }

  onMount(() => {
    // A press of the pointer ends the key's window: a right click right after the key is a click, not its echo.
    const pointer = () => (menuKeyAt = -Infinity)
    window.addEventListener('contextmenu', dropEcho, true)
    window.addEventListener('pointerdown', pointer, true)
    return () => {
      window.removeEventListener('contextmenu', dropEcho, true)
      window.removeEventListener('pointerdown', pointer, true)
    }
  })

  function onKeyDown(event: KeyboardEvent) {
    if (isMenuKey(event)) return askMenu(event)
    const save = saveShortcut(event)
    if (save) {
      // The browser would offer to save the page as a file.
      event.preventDefault()
      dispatcher.run(save === 'save' ? saveCommand() : saveAsCommand())
      return
    }
    const layout = layoutShortcut(event)
    // No layout shortcut while a menu or a dialog has the keyboard (the pairing field is a text field and is left out already).
    // Nor while a modal dialog is open, whatever holds the focus (a click on its text leaves it on the body).
    if (layout && !(event.target as Element | null)?.closest?.('[role="dialog"], [role="menu"]') && !document.querySelector('[aria-modal="true"]')) {
      event.preventDefault()
      if (layout === 'panel') ui.toggleMaximized()
      else {
        const view = viewsIn('main')[layout.view]
        if (view) ui.show(view.id)
      }
      return
    }
    // `?` opens the shortcut list, and closes it again; no other dialog or menu has to be open (#474).
    if (helpShortcut(event)) {
      const other = document.querySelector('[aria-modal="true"]:not([data-testid="shortcuts-dialog"])')
      if (ui.helpOpen || (!other && !(event.target as Element | null)?.closest?.('[role="menu"]'))) {
        event.preventDefault()
        ui.helpOpen = !ui.helpOpen
        return
      }
    }
    // The open list holds the keyboard: no undo, redo or note behind it (the dialog keeps Escape and Tab itself).
    if (ui.helpOpen) return
    const action = historyShortcut(event)
    if (!action) {
      // The computer keyboard plays in every view; what is no shortcut of the shell goes to the notes.
      keys.keyDown(event)
      return
    }
    event.preventDefault()
    // Undo pressed during a fader drag (a history group is open) does nothing, and says nothing.
    runHistory(dispatcher, ui, action, { quiet: true })
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

  // The skip link (#471): the first stop of the keyboard, past the top bar and the track column, to the timeline (in the
  // arrangement) or to the main view. It moves the focus itself: a hash alone would not reach a canvas widget.
  let mainArea = $state<HTMLElement | null>(null)
  function skipToMain(event: MouseEvent) {
    event.preventDefault()
    const target = mainArea?.querySelector<HTMLElement>('[data-testid="timegrid"]') ?? mainArea
    target?.focus()
  }

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

<svelte:window
  oncontextmenu={onContextMenu}
  onkeydown={onKeyDown}
  onkeyup={(event) => keys.keyUp(event)}
  onbeforeunload={onBeforeUnload}
  onfocus={() => void storage.wake()}
/>
<svelte:document onvisibilitychange={onVisibility} />

<div class="shell">
  <a class="skip" href="#fk-main" data-testid="skip-link" onclick={skipToMain}>{ui.t(ui.view === 'arrangement' ? 'shell.skipTimeline' : 'shell.skipMain')}</a>
  <TopBar />
  <StorageBanners />
  {#if ui.contextMenu}
    {#key ui.contextMenu}
      <ContextMenu x={ui.contextMenu.x} y={ui.contextMenu.y} items={ui.contextMenu.items} label={ui.contextMenu.label} onclose={() => ui.closeMenu()} />
    {/key}
  {/if}
  <ExportDialog />
  <EngineDialog />
  <ShortcutsDialog />
  {#if engine.notice}
    <p class="notice" role="status">{ui.t(engine.notice)}</p>
  {/if}
  {#if engine.streamError}
    <p class="notice" role="alert" data-testid="stream-error">{ui.t('engine.streamError')}</p>
  {/if}

  {#if engine.pendingPairCode && !engine.started && !ui.engineOpen}
    <p class="notice" role="status" data-testid="pair-pending-notice">{ui.t('start.pairPending')}</p>
  {/if}
  {#if engine.pairingMessage && !ui.engineOpen}
    <p class="notice" role="status" data-testid="pairing-notice">{ui.t(engine.pairingMessage.key, { n: engine.pairingMessage.n ?? 0 })}</p>
  {/if}
  {#if engine.refusal}
    {@const text = ui.t(engine.refusal.key, { ...engine.refusal.vars, what: engine.refusal.whatKey ? ui.t(engine.refusal.whatKey) : '' })}
    <p class="toast" role="status" data-testid="refusal-toast">{engine.refusal.count > 1 ? ui.t('refusal.count', { text, n: engine.refusal.count }) : text}</p>
  {/if}
  <div class="body" data-panel={ui.panel} bind:clientHeight={bodyHeight} style:--panel-row={panelRow}>
    <TrackList />
    <main class="main" id="fk-main" tabindex="-1" bind:this={mainArea} aria-label={ui.t(current.label)} data-testid="view" data-view={current.id}>
      {#key ui.layoutRev}<current.component />{/key}
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
  /* Out of sight until the keyboard reaches it; then over the top bar's left corner, in the focus colour. */
  .skip {
    position: fixed;
    top: var(--fk-space-2);
    left: var(--fk-space-2);
    z-index: 60;
    padding: var(--fk-space-2) var(--fk-space-3);
    border-radius: var(--fk-radius-pill);
    background: var(--fk-bg);
    color: var(--fk-text);
    box-shadow: var(--fk-shadow-float);
    font-weight: 600;
    text-decoration: none;
    transform: translateY(calc(-100% - var(--fk-space-4)));
  }
  .skip:focus-visible {
    transform: none;
    outline: 2px solid var(--fk-accent-fg);
    outline-offset: 2px;
  }
  .main:focus {
    outline: none;
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
    grid-template-rows: minmax(0, 1fr) var(--panel-row, minmax(392px, 1.2fr));
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
