<script lang="ts">
  /**
   * A continuous control: the one behaviour of every knob, fader and pan (`continuous.ts`). A
   * `role="slider"` element, not a native range input, so the drag has a magnet on the default value
   * and a fine mode (Ctrl, Cmd on macOS), a double click or the context menu puts the default value
   * back as one command, and the keyboard (arrows, Ctrl + arrows, Page keys, Home, End, Delete) works
   * on a focused control. Enter, or `edit()` (the readout next to the control), opens a field to type
   * the value (`entry`, `ValueField.svelte`); the owner reads the text and sends the command. The position is 0 to 1; the owner converts it to its unit, sends the command
   * and groups a drag as one history entry with `onbegin` and `onend` (`gesture.ts`). It holds no
   * project state.
   */
  import type { Snippet } from 'svelte'
  import { createDrag, DRAG_SLOP_PX, fineKey, keyAction, type Drag } from '../continuous'
  import { useStores } from '../stores'
  import ValueField from './ValueField.svelte'
  import ValueMenu from './ValueMenu.svelte'

  /** The thumb of a fader or a pan, in pixels: the travel is the control's length minus this. */
  const THUMB = 16
  /** The pointer travel of a knob's whole range, in pixels. */
  const KNOB_TRAVEL = 160

  let {
    kind = 'fader',
    orientation = 'horizontal',
    label,
    position,
    home,
    valueText,
    defaultText,
    travel,
    hint,
    testid,
    onposition,
    onreset,
    entry,
    onbegin,
    onend,
    children,
  }: {
    /** `fader` draws a rail and a thumb, `pan` fills from the centre, `knob` draws what `children` gives. */
    kind?: 'fader' | 'pan' | 'knob'
    orientation?: 'horizontal' | 'vertical'
    label: string
    /** 0 to 1. */
    position: number
    /** The default value's position, held by the magnet; null for none. */
    home: number | null
    /** The value as text, announced by screen readers. */
    valueText: string
    /** The default value as text, shown in the context menu. */
    defaultText: string
    /** The pointer travel for the whole range, in pixels; by default 160 for a knob and the control's length minus the thumb otherwise. */
    travel?: number
    hint?: string
    testid?: string
    onposition: (position: number) => void
    /** Puts the default value back, as one command. */
    onreset: () => void
    /** Typing a value: the text the field opens with (the displayed value) and what applies a text (true when accepted). Without it the control has no field. */
    entry?: { text: () => string; commit: (text: string) => boolean }
    /** A drag starts and ends: the owner groups it as one history entry. */
    onbegin?: () => void
    onend?: () => void
    children?: Snippet
  } = $props()

  const { ui } = useStores()
  const vertical = $derived(orientation === 'vertical')

  let root: HTMLDivElement | undefined = $state()
  let menu = $state<{ x: number; y: number } | null>(null)
  // The text the value field opened with; null while it is closed.
  let typing = $state<string | null>(null)

  /** Opens the field to type a value: Enter on the focused control, and the readouts next to it, call it. */
  export function edit() {
    if (!entry || pointer || menu || typing !== null) return
    typing = entry.text()
  }

  // The drag in progress, from the press to the release.
  let pointer: { id: number; x: number; y: number } | null = null
  let started = false
  let began = false
  // The last press moved the control: a double click that ends with such a press must not reset what it just set.
  let moved = false
  let drag: Drag | null = null
  let offset = 0

  /** The pointer travel since the press that raises the value: up for a vertical control, right for a horizontal one, either for a knob. */
  function travelOf(event: PointerEvent): number {
    if (!pointer) return 0
    const dx = event.clientX - pointer.x
    const dy = pointer.y - event.clientY
    return kind === 'knob' ? dx + dy : vertical ? dy : dx
  }

  function onPointerDown(event: PointerEvent) {
    if (event.button !== 0 || menu) return
    root?.setPointerCapture?.(event.pointerId)
    pointer = { id: event.pointerId, x: event.clientX, y: event.clientY }
    started = false
    began = false
    drag = null
    keyed = null
    moved = false
  }

  function onPointerMove(event: PointerEvent) {
    if (!pointer || event.pointerId !== pointer.id) return
    const px = travelOf(event)
    if (!started) {
      if (Math.hypot(event.clientX - pointer.x, event.clientY - pointer.y) < DRAG_SLOP_PX) return
      started = true
      moved = true
      offset = px
      const length = travel ?? (kind === 'knob' ? KNOB_TRAVEL : Math.max(1, ((vertical ? root?.clientHeight : root?.clientWidth) ?? THUMB + 1) - THUMB))
      drag = createDrag({ position, home, travel: length })
    }
    const next = drag!.move(px - offset, fineKey(event))
    // The history group opens at the first change, not at the press: a drag that changes nothing leaves no entry.
    if (!began) {
      if (next === position) return
      began = true
      onbegin?.()
    }
    onposition(next)
  }

  function stop(event: PointerEvent) {
    if (!pointer || event.pointerId !== pointer.id) return
    pointer = null
    drag = null
    if (began) onend?.()
    started = false
    began = false
  }

  // The last position a key sent. The owner rounds the value (0.1 dB on a fader), so the position it
  // hands back differs a little from what was sent: stepping from that would drift. Within the
  // tolerance the key steps from what it sent; a change from elsewhere (undo, a drag) is far enough.
  let keyed: { sent: number } | null = null
  const KEY_TOLERANCE = 0.004

  function onKeyDown(event: KeyboardEvent) {
    // Not while a drag is held: a reset must never land inside its history group.
    if (event.target !== event.currentTarget || event.altKey || pointer) return
    if (event.key === 'Enter' && entry && !event.ctrlKey && !event.metaKey && !event.shiftKey) {
      event.preventDefault()
      event.stopPropagation()
      // A held key does not reopen the field the instant it closes.
      if (!event.repeat) edit()
      return
    }
    const base = keyed && Math.abs(position - keyed.sent) < KEY_TOLERANCE ? keyed.sent : position
    const action = keyAction(event.key, { position: base, home, fine: fineKey(event) })
    if (!action) return
    event.preventDefault()
    event.stopPropagation()
    if ('reset' in action) {
      keyed = null
      onreset()
    } else {
      keyed = { sent: action.position }
      onposition(action.position)
    }
  }

  function onContextMenu(event: MouseEvent) {
    event.preventDefault()
    // Keep the menu on screen.
    menu = { x: Math.max(0, Math.min(event.clientX, window.innerWidth - 220)), y: Math.max(0, Math.min(event.clientY, window.innerHeight - 56)) }
  }

  function closeMenu(refocus = true) {
    menu = null
    if (refocus) root?.focus()
  }

  const along = (p: number) => `calc(${p} * (100% - ${THUMB}px))`
</script>

<div
  bind:this={root}
  class="slider {kind} {orientation}"
  role="slider"
  tabindex="0"
  title={hint ?? ui.t('controls.hint')}
  aria-label={label}
  aria-orientation={orientation}
  aria-valuemin="0"
  aria-valuemax="1"
  aria-valuenow={position}
  aria-valuetext={valueText}
  data-testid={testid}
  data-travel={travel}
  onpointerdown={onPointerDown}
  onpointermove={onPointerMove}
  onpointerup={stop}
  onpointercancel={stop}
  onlostpointercapture={stop}
  onblur={() => {
    if (began) onend?.()
    started = false
    began = false
    pointer = null
  }}
  ondblclick={() => {
    if (!moved) onreset()
  }}
  oncontextmenu={onContextMenu}
  onkeydown={onKeyDown}
>
  {#if kind === 'knob'}
    {@render children?.()}
  {:else}
    <span class="rail"></span>
    {#if kind === 'pan'}
      <span
        class="fill"
        style:left="calc({Math.min(position, 0.5)} * (100% - {THUMB}px) + {THUMB / 2}px)"
        style:width="calc({Math.abs(position - 0.5)} * (100% - {THUMB}px))"
      ></span>
    {:else if vertical}
      <span class="fill" style:height="calc({along(position)} + {THUMB / 2}px)"></span>
    {:else}
      <span class="fill" style:width="calc({along(position)} + {THUMB / 2}px)"></span>
    {/if}
    {#if home !== null}
      <span class="home" style={vertical ? `bottom: calc(${along(home)} + ${THUMB / 2}px)` : `left: calc(${along(home)} + ${THUMB / 2}px)`}></span>
    {/if}
    <span class="thumb" style={vertical ? `bottom: ${along(position)}` : `left: ${along(position)}`}></span>
  {/if}
</div>

{#if typing !== null && root}
  <ValueField
    anchor={root}
    initial={typing}
    {label}
    commit={(text) => entry?.commit(text) ?? false}
    onclose={(refocus) => {
      typing = null
      if (refocus) root?.focus()
    }}
  />
{/if}

{#if menu}
  <ValueMenu
    x={menu.x}
    y={menu.y}
    value={defaultText}
    onreset={() => {
      closeMenu()
      onreset()
    }}
    onclose={() => closeMenu()}
  />
{/if}

<style>
  .slider {
    position: relative;
    flex: none;
    touch-action: none;
    user-select: none;
    cursor: pointer;
  }
  .slider.horizontal:not(.knob) {
    flex: 1 1 auto;
    min-width: 0;
    width: 100%;
    height: var(--fk-row);
  }
  .slider.vertical {
    width: 22px;
    height: 100%;
  }
  .slider.knob {
    width: 48px;
    height: 48px;
    cursor: ns-resize;
  }
  .slider:focus-visible {
    outline: 2px solid var(--fk-accent-fg);
    outline-offset: 2px;
    border-radius: var(--fk-radius-work);
  }
  .rail,
  .fill {
    position: absolute;
    border-radius: 2px;
  }
  .rail {
    background: var(--fk-line);
  }
  .fill {
    background: var(--fk-accent-fg);
  }
  .horizontal .rail,
  .horizontal .fill {
    top: calc(50% - 2px);
    height: 4px;
  }
  .horizontal .rail {
    left: 0;
    right: 0;
  }
  .horizontal.fader .fill {
    left: 0;
  }
  .vertical .rail,
  .vertical .fill {
    left: calc(50% - 2px);
    width: 4px;
  }
  .vertical .rail {
    top: 0;
    bottom: 0;
  }
  .vertical .fill {
    bottom: 0;
  }
  .home {
    position: absolute;
    background: var(--fk-text-muted);
  }
  .horizontal .home {
    top: calc(50% - 6px);
    width: 1px;
    height: 12px;
  }
  .vertical .home {
    left: calc(50% - 6px);
    width: 12px;
    height: 1px;
  }
  .thumb {
    position: absolute;
    border: 2px solid var(--fk-accent-fg);
    border-radius: var(--fk-radius-work);
    background: var(--fk-bg);
    box-sizing: border-box;
  }
  .horizontal .thumb {
    top: calc(50% - 8px);
    width: 16px;
    height: 16px;
  }
  .vertical .thumb {
    left: 2px;
    right: 2px;
    height: 16px;
  }
</style>
