<script lang="ts">
  /**
   * The colour of a track, chosen from the sixteen hues of the palette (`TRACK_COLORS`). The swatch
   * opens a grid of named colours: arrows move through it (left and right by one, up and down by a
   * row), Home and End go to the ends, Enter or Space picks, Escape closes. Each swatch is a 24 px
   * target and has its colour's name. The picker only reports the pick (`onpick`); the owner sends
   * the command.
   */
  import { tick } from 'svelte'
  import { HUE_KEYS, hueKey, TRACK_COLORS } from '../project/tracks'
  import { useStores } from '../stores'

  let {
    color,
    stored = null,
    name,
    open = $bindable(false),
    onpick,
  }: {
    /** The colour shown now (the track's, from the palette or not). */
    color: string
    /** The colour kept in the project, null when the track has none: picking the hue shown then stores it. */
    stored?: string | null
    /** The track's name, for the accessible names. */
    name: string
    open?: boolean
    onpick: (color: string) => void
  } = $props()

  const { ui } = useStores()
  const t = ui.t
  const COLUMNS = 4

  let root = $state<HTMLElement | null>(null)
  let trigger = $state<HTMLButtonElement | null>(null)
  let cells = $state<HTMLButtonElement[]>([])
  /** The swatch that takes the Tab stop: the track's colour when it is in the palette, else the first. */
  let active = $state(0)

  const current = $derived(TRACK_COLORS.findIndex((c) => c === color.toLowerCase()))
  const currentName = $derived.by(() => {
    const key = hueKey(color)
    return key ? t(key) : null
  })

  async function toggle() {
    open = !open
    if (!open) return
    active = Math.max(0, current)
    await tick()
    cells[active]?.focus()
  }

  function close(restore: boolean) {
    if (!open) return
    open = false
    if (restore) trigger?.focus()
  }

  function pick(hue: string) {
    close(true)
    if (hue !== stored?.toLowerCase()) onpick(hue)
  }

  async function focusCell(index: number) {
    active = Math.min(TRACK_COLORS.length - 1, Math.max(0, index))
    await tick()
    cells[active]?.focus()
  }

  function onKey(event: KeyboardEvent) {
    if (event.key === 'Escape') {
      event.preventDefault()
      event.stopPropagation()
      close(true)
      return
    }
    const step: Record<string, number> = { ArrowLeft: -1, ArrowRight: 1, ArrowUp: -COLUMNS, ArrowDown: COLUMNS }
    if (event.key in step) {
      event.preventDefault()
      void focusCell(active + step[event.key])
    } else if (event.key === 'Home' || event.key === 'End') {
      event.preventDefault()
      void focusCell(event.key === 'Home' ? 0 : TRACK_COLORS.length - 1)
    } else if (event.key === 'Tab') {
      // Tab leaves the grid: it is one stop, like a menu.
      close(false)
    }
  }

  function onWindowPointer(event: PointerEvent) {
    if (open && !root?.contains(event.target as Node)) close(false)
  }
</script>

<svelte:window onpointerdown={onWindowPointer} />

<div class="swatch" bind:this={root}>
  <button
    class="trigger"
    type="button"
    bind:this={trigger}
    aria-haspopup="true"
    aria-expanded={open}
    aria-label={currentName ? `${t('tracks.color', { name })}: ${currentName}` : t('tracks.color', { name })}
    title={t('tracks.color', { name })}
    data-testid="track-color"
    onclick={toggle}
  >
    <span class="chip" style:background={color}></span>
  </button>
  {#if open}
    <div class="palette" role="radiogroup" aria-label={t('tracks.colorPalette')} data-testid="track-palette" tabindex="-1" onkeydown={onKey}>
      {#each TRACK_COLORS as hue, index (hue)}
        <button
          class="hue"
          type="button"
          role="radio"
          aria-checked={index === current}
          aria-label={t(HUE_KEYS[index])}
          title={t(HUE_KEYS[index])}
          tabindex={index === active ? 0 : -1}
          style:background={hue}
          data-testid="track-hue"
          bind:this={cells[index]}
          onclick={() => pick(hue)}
        >
          {#if index === current}
            <svg width="12" height="12" viewBox="0 0 12 12" aria-hidden="true"
              ><path d="M2.5 6.5 5 9l4.5-5.5" fill="none" stroke="var(--fk-on-signal)" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" /></svg
            >
          {/if}
        </button>
      {/each}
    </div>
  {/if}
</div>

<style>
  .swatch {
    position: relative;
    flex: none;
  }
  /* A 24 px target around the 14 px chip. */
  .trigger {
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    padding: 0;
    border: 0;
    border-radius: var(--fk-radius-work);
    background: transparent;
    cursor: pointer;
  }
  .trigger:hover {
    background: var(--fk-surface-2);
  }
  .chip {
    width: 14px;
    height: 14px;
    border-radius: var(--fk-radius-work);
  }
  .palette {
    position: absolute;
    z-index: 7;
    top: 28px;
    left: 0;
    display: grid;
    grid-template-columns: repeat(4, 24px);
    gap: 4px;
    padding: var(--fk-space-2);
    border: 1px solid var(--fk-line);
    border-radius: var(--fk-radius-panel);
    background: var(--fk-bg);
    box-shadow: var(--fk-shadow-float);
  }
  .hue {
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    padding: 0;
    border: 1px solid transparent;
    border-radius: var(--fk-radius-work);
    cursor: pointer;
  }
  .hue:hover {
    border-color: var(--fk-text);
  }
</style>
