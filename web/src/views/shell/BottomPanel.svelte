<script lang="ts">
  // The bottom panel: shows the registry's `panel` views (devices, piano roll), switched by its tabs.
  // Its top edge is a handle: drag or arrow keys resize it (the height is kept per view, in the page's
  // preferences), a double click or Enter folds it to its tabs, as does the button; Alt+M (or the button) maximises it.
  // An action that targets the panel (`ui.showPanel`) unfolds it and the focus comes to its body. In the mixer the panel
  // is folded to its tabs (the mixer needs the height), and unfolds for the visit when asked.
  import { useStores } from '../../lib/stores'
  import { viewById, viewsIn, type PanelViewId } from '../registry'
  import DevicesView from '../devices/DevicesView.svelte'
  import { BIG_STEP, PANEL_MIN, SPLIT_MIN_WIDTH, STEP } from './panel-size'

  let { max }: { /** The tallest the panel may be in the room there is (px). */ max: number } = $props()

  const { ui } = useStores()
  const tabs = viewsIn('panel')
  const current = $derived(viewById(ui.panel) ?? tabs[0])
  const id = $derived(current.id as PanelViewId)
  const min = $derived(PANEL_MIN[id])
  const collapsed = $derived(ui.panelFolded)
  const maximized = $derived(ui.panelMaximized && !collapsed)

  let height = $state(0)
  let width = $state(0)
  let bodyEl = $state<HTMLElement | null>(null)
  /** The roll and the devices side by side when the panel is wide enough for both (the Devices tab stays for the full chain). */
  const split = $derived(current.id === 'piano-roll' && width >= SPLIT_MIN_WIDTH)

  // An action that targets the panel (it unfolds it) brings the focus into it. The body is not a control: the keys still play.
  let seenFocus = ui.panelFocus
  $effect(() => {
    const asked = ui.panelFocus
    if (asked === seenFocus) return
    seenFocus = asked
    queueMicrotask(() => bodyEl?.focus({ preventScroll: true }))
  })
  /** Where a step starts: the height last asked for, else the one measured (a key pressed twice before the layout settles adds up). */
  const base = () => Math.min(max, Math.max(min, ui.panelHeights[id] ?? height))
  let dragging = $state(false)
  let drag: { y: number; height: number } | null = null

  /** The height the panel shows, for the handle's value: 0 when folded. */
  const shown = $derived(collapsed ? 0 : Math.round(height))

  function resize(next: number) {
    ui.setPanelHeight(id, Math.round(Math.min(max, Math.max(min, next))))
  }

  function onPointerDown(event: PointerEvent) {
    if (event.button !== 0 || collapsed) return
    drag = { y: event.clientY, height: base() }
    dragging = true
    ;(event.currentTarget as HTMLElement).setPointerCapture(event.pointerId)
    event.preventDefault()
  }
  function onPointerMove(event: PointerEvent) {
    if (drag) resize(drag.height + drag.y - event.clientY)
  }
  function endDrag() {
    drag = null
    dragging = false
  }

  function onKeyDown(event: KeyboardEvent) {
    if (event.ctrlKey || event.metaKey || event.altKey) return
    const step = event.shiftKey ? BIG_STEP : STEP
    let handled = true
    if (event.key === 'Enter') ui.toggleFold()
    else if (event.key === 'ArrowUp') {
      if (collapsed) ui.unfoldPanel()
      else resize(base() + step)
    } else if (event.key === 'ArrowDown') {
      if (!collapsed) resize(base() - step)
    } else if (event.key === 'Home') {
      if (!collapsed) resize(min)
    } else if (event.key === 'End') {
      if (collapsed) ui.unfoldPanel()
      resize(max)
    } else handled = false
    if (handled) event.preventDefault()
  }
</script>

<section
  class="panel"
  class:collapsed
  aria-label={ui.t('panel.label')}
  data-testid="bottom-panel"
  data-panel={current.id}
  data-collapsed={collapsed}
  bind:clientHeight={height}
>
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions, a11y_no_noninteractive_tabindex -->
  <div
    class="handle"
    class:dragging
    role="separator"
    tabindex="0"
    aria-orientation="horizontal"
    aria-label={ui.t('panel.resize')}
    aria-valuemin={collapsed ? 0 : min}
    aria-valuemax={max}
    aria-valuenow={shown}
    data-testid="bottom-panel-handle"
    onpointerdown={onPointerDown}
    onpointermove={onPointerMove}
    onpointerup={endDrag}
    onpointercancel={endDrag}
    onlostpointercapture={endDrag}
    ondblclick={() => ui.toggleFold()}
    onkeydown={onKeyDown}
  ></div>
  <div class="tabs">
    {#each tabs as tab (tab.id)}
      <button aria-pressed={ui.panel === tab.id} data-testid="panel-{tab.id}" onclick={() => ui.showPanel(tab.id)}>{ui.t(tab.label)}</button>
    {/each}
    <button
      class="maximize"
      aria-pressed={maximized}
      aria-keyshortcuts="Alt+M"
      aria-label={ui.t(maximized ? 'panel.restore' : 'panel.maximize')}
      title={ui.t(maximized ? 'panel.restore' : 'panel.maximize')}
      data-testid="bottom-panel-maximize"
      onclick={() => ui.toggleMaximized()}
    >
      <svg viewBox="0 0 12 12" width="12" height="12" aria-hidden="true"
        ><path d={maximized ? 'M2 2h8v8H2z M2 5h8' : 'M2 2h8v8H2z'} fill="none" stroke="currentColor" stroke-width="1.4" /></svg
      >
    </button>
    <button
      class="fold"
      aria-expanded={!collapsed}
      aria-label={ui.t(collapsed ? 'panel.expand' : 'panel.collapse')}
      title={ui.t(collapsed ? 'panel.expand' : 'panel.collapse')}
      data-testid="bottom-panel-fold"
      onclick={() => ui.toggleFold()}
    >
      <svg viewBox="0 0 12 12" width="12" height="12" aria-hidden="true"
        ><path d={collapsed ? 'M2 8 6 4l4 4' : 'M2 4l4 4 4-4'} fill="none" stroke="currentColor" stroke-width="1.5" /></svg
      >
    </button>
  </div>
  {#if !collapsed}
    <div class="body" class:split bind:this={bodyEl} tabindex="-1" data-testid="bottom-panel-body" bind:clientWidth={width}>
      {#if split}
        <div class="main"><current.component /></div>
        <div class="side" data-testid="roll-devices"><DevicesView compact /></div>
      {:else}
        <current.component />
      {/if}
    </div>
  {/if}
</section>

<style>
  .panel {
    grid-area: panel;
    position: relative;
    min-height: 0;
    display: flex;
    flex-direction: column;
    border-top: 1px solid var(--fk-line);
    background: var(--fk-surface);
  }
  /* A wider hit area than the line it sits on; it straddles the panel's top edge. */
  .handle {
    position: absolute;
    top: -4px;
    left: 0;
    right: 0;
    height: 8px;
    z-index: 2;
    cursor: ns-resize;
    touch-action: none;
  }
  .handle:hover,
  .handle.dragging,
  .handle:focus-visible {
    background: var(--fk-line);
  }
  /* The keyboard's resize handle shows where the focus is. */
  .handle:focus-visible {
    outline: 2px solid var(--fk-accent-fg);
    outline-offset: -2px;
  }
  .collapsed .handle {
    cursor: pointer;
  }
  .tabs {
    display: flex;
    gap: var(--fk-space-1);
    padding: var(--fk-space-2) var(--fk-space-3);
    border-bottom: 1px solid var(--fk-line);
  }
  button {
    height: 26px;
    padding: 0 var(--fk-space-3);
    border: 0;
    border-radius: var(--fk-radius-pill);
    background: transparent;
    color: var(--fk-text-muted);
    font-weight: 500;
    font-size: 12px;
    cursor: pointer;
  }
  button:hover {
    color: var(--fk-text);
  }
  button[aria-pressed='true'] {
    background: var(--fk-surface-2);
    color: var(--fk-text);
  }
  .maximize {
    margin-left: auto;
    display: grid;
    place-items: center;
    width: 26px;
    padding: 0;
  }
  .fold {
    display: grid;
    place-items: center;
    width: 26px;
    padding: 0;
  }
  .body {
    flex: 1;
    min-height: 0;
    min-width: 0;
    overflow: auto;
  }
  .body.split {
    display: flex;
  }
  .body:focus-visible {
    outline: 2px solid var(--fk-accent-fg);
    outline-offset: -2px;
  }
  .main {
    flex: 1;
    min-width: 0;
    min-height: 0;
    overflow: auto;
  }
  .main > :global(*) {
    height: 100%;
  }
  /* The roll keeps the room; the devices take a column of the chain's own width beside it. */
  .side {
    flex: none;
    width: 300px;
    min-height: 0;
    overflow: auto;
    border-left: 1px solid var(--fk-line);
  }
  @media (max-width: 760px) {
    .handle {
      display: none;
    }
  }
</style>
