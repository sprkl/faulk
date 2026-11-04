<script lang="ts">
  /**
   * The project's key and scale in the transport bar (#695): a short button ("Fm", "D dor.") that opens a small panel
   * with the root and the scale. Each choice is one `project.set_scale` through the dispatcher; the panel holds no state
   * of its own but whether it is open. Escape, a click outside or leaving the panel closes it; the focus goes back to the
   * button when it closed from the keyboard. When the top bar is full the button folds away (`folded`, step 3 of #604's
   * order, `TopBar.svelte`): the bar's "more" menu opens the same panel, which then gives the focus back to the
   * "more" button.
   */
  import { useStores } from '../../lib/stores'
  import { setScale } from '../../lib/transport-commands'
  import { kindOf, rootLabel, ROOTS, SCALE_MODES, scaleName, scaleShort } from '../../lib/scale'
  import type { ScaleKind } from '../../lib/generated/protocol'

  let { disabled = false, folded = false }: { disabled?: boolean; folded?: boolean } = $props()

  const { project, ui, dispatcher } = useStores()
  const t = ui.t

  const scale = $derived(project.view?.project.scale ?? null)
  const current = $derived(kindOf(scale))
  const name = $derived(scaleName(scale, t))

  let button = $state<HTMLButtonElement | null>(null)
  let panel = $state<HTMLElement | null>(null)
  let rootSelect = $state<HTMLSelectElement | null>(null)

  const open = $derived(ui.scaleOpen && !disabled)

  // After the menu that may have opened the panel gave the focus back to its opener (a microtask): the root takes it.
  $effect(() => {
    if (!open) return
    const timer = setTimeout(() => rootSelect?.focus({ preventScroll: true }))
    return () => clearTimeout(timer)
  })

  /** The control that opens the panel where the bar shows it: the button, or the "more" button that holds it when folded. */
  const opener = () => (folded ? document.querySelector<HTMLElement>('[data-testid="bar-more"]') : button)

  function close(refocus: boolean) {
    ui.scaleOpen = false
    if (refocus) opener()?.focus({ preventScroll: true })
  }

  /** A root chosen with no scale yet picks the major scale on it: a root alone names no scale. */
  function onRoot(event: Event) {
    const root = Number((event.currentTarget as HTMLSelectElement).value)
    const mode: ScaleKind = current.mode === 'chromatic' ? 'major' : current.mode
    if (root !== current.root || mode !== current.mode) void dispatcher.run(setScale(root, mode))
  }

  function onMode(event: Event) {
    const value = (event.currentTarget as HTMLSelectElement).value
    const mode = value === 'chromatic' ? 'chromatic' : SCALE_MODES.find((m) => m === value)
    if (mode && mode !== current.mode) void dispatcher.run(setScale(mode === 'chromatic' ? 0 : current.root, mode))
  }

  function onKey(event: KeyboardEvent) {
    if (event.key !== 'Escape') return
    event.preventDefault()
    event.stopPropagation()
    close(true)
  }

  /** Leaving the panel (Tab out, a click elsewhere) closes it; moving between its own controls does not. */
  function onFocusOut(event: FocusEvent) {
    const next = event.relatedTarget as Node | null
    if (next && (panel?.contains(next) || next === button)) return
    ui.scaleOpen = false
  }

  function onWindowPointer(event: PointerEvent) {
    if (!open) return
    const target = event.target as Node | null
    if (target && (panel?.contains(target) || button?.contains(target))) return
    // The "more" menu's entry opens it: the press on the menu is not a press outside.
    if ((target as Element | null)?.closest?.('[data-context-menu]')) return
    ui.scaleOpen = false
  }
</script>

<svelte:window onpointerdown={onWindowPointer} />

<!-- Folded and closed, the wrapper takes no room in the bar; open, it is the anchor of the panel, at the end of the transport. -->
<div class="scale" class:folded hidden={folded && !open}>
  <button
    class="value pick"
    class:none={!scale}
    bind:this={button}
    type="button"
    {disabled}
    aria-haspopup="dialog"
    aria-expanded={open}
    aria-label={t('scale.title', { name })}
    title={`${t('scale.title', { name })}. ${t('scale.hint')}`}
    data-testid="transport-scale"
    data-root={current.root}
    data-mode={current.mode}
    hidden={folded}
    data-folds
    onclick={() => (open ? close(false) : (ui.scaleOpen = true))}>{scaleShort(scale, t)}</button
  >
  {#if open}
    <div
      class="panel"
      role="dialog"
      aria-label={t('scale.label')}
      bind:this={panel}
      data-testid="scale-panel"
      tabindex="-1"
      onkeydown={onKey}
      onfocusout={onFocusOut}
    >
      <label>
        <span>{t('scale.root')}</span>
        <select bind:this={rootSelect} value={String(current.root)} data-testid="scale-root" onchange={onRoot}>
          {#each ROOTS as root (root)}<option value={String(root)}>{rootLabel(root, t)}</option>{/each}
        </select>
      </label>
      <label>
        <span>{t('scale.mode')}</span>
        <select value={current.mode} data-testid="scale-mode" onchange={onMode}>
          <option value="chromatic">{t('scale.none')}</option>
          {#each SCALE_MODES as mode (mode)}<option value={mode}>{t(`scale.mode.${mode}`)}</option>{/each}
        </select>
      </label>
    </div>
  {/if}
</div>

<style>
  .scale {
    position: relative;
    display: inline-flex;
    margin-left: var(--fk-space-1);
  }
  .scale[hidden] {
    display: none;
  }
  .scale.folded {
    margin-left: 0;
  }
  /* The width at which the transport's fields tighten (`Transport.svelte`). */
  @media (max-width: 1399px) {
    .scale {
      margin-left: 2px;
    }
  }
  .pick {
    height: 28px;
    padding: 0 6px;
    border: 1px solid var(--fk-line);
    border-radius: var(--fk-radius-panel);
    background: var(--fk-bg);
    color: var(--fk-text);
    font-size: 12px;
    white-space: nowrap;
    cursor: pointer;
  }
  .pick.none {
    color: var(--fk-text-muted);
    font-size: 11px;
  }
  .pick[hidden] {
    display: none;
  }
  .pick:hover:not(:disabled) {
    border-color: var(--fk-text-muted);
  }
  .pick:disabled {
    opacity: 0.45;
    cursor: default;
  }
  .panel {
    position: absolute;
    top: calc(100% + 4px);
    right: 0;
    z-index: 30;
    display: flex;
    flex-direction: column;
    gap: var(--fk-space-2);
    width: max-content;
    padding: var(--fk-space-3);
    border: 1px solid var(--fk-line);
    border-radius: var(--fk-radius-panel);
    background: var(--fk-surface);
    box-shadow: var(--fk-shadow-float);
  }
  .panel label {
    display: grid;
    grid-template-columns: 64px 1fr;
    align-items: center;
    gap: var(--fk-space-2);
    font-size: 12px;
    color: var(--fk-text-muted);
  }
  .panel select {
    height: 28px;
    min-width: 160px;
    border: 1px solid var(--fk-line);
    border-radius: var(--fk-radius-panel);
    background: var(--fk-bg);
    color: var(--fk-text);
    font-size: 13px;
    padding: 0 var(--fk-space-2);
  }
</style>
