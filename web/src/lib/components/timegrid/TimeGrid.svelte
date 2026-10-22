<script lang="ts">
  /**
   * The shared time grid: a ruler (bars and beats) over a body of lines, drawn on one canvas, with
   * zoom, horizontal scroll and snapping. It draws and forwards input; the maths is in `grid.ts`.
   * Pointer ticks reach the host already snapped (`onhover`, `onseek`), so a clip drag, a note
   * drag or an automation point can use the same conversion. Zoom and scroll are local view
   * state, not commands.
   *
   * Input: Ctrl or Cmd + wheel zooms around the pointer; Shift + wheel (or a horizontal wheel)
   * scrolls; a plain vertical wheel is left to the host (tracks scroll vertically). With the grid
   * focused: arrows scroll (Shift for a page), + and - zoom, Home goes to the start.
   */
  import type { Signature } from '../../../views/transport/time'
  import {
    DEFAULT_PX_PER_TICK,
    DEFAULT_TOTAL_BARS,
    clampView,
    scrollBy,
    snapStep,
    snapTick,
    totalTicksOf,
    xToTick,
    zoomAt,
    type SnapSetting,
    type View,
  } from './grid'
  import { RULER_HEIGHT, drawGrid, readColors, type BodyFrame, type Colors } from './render'

  let {
    signature,
    totalBars = DEFAULT_TOTAL_BARS,
    view = $bindable({ pxPerTick: DEFAULT_PX_PER_TICK, scrollTick: 0 }),
    snap = 'auto',
    playhead = null,
    label,
    onseek,
    onhover,
    drawBody,
  }: {
    signature: Signature
    totalBars?: number
    view?: View
    snap?: SnapSetting
    /** The play position in ticks, or `null` to draw none. */
    playhead?: number | null
    label: string
    /** A click on the ruler, with the snapped tick. */
    onseek?: (tick: number) => void
    /** The snapped tick under the pointer, `null` when it leaves. */
    onhover?: (tick: number | null) => void
    /**
     * Draws the host's content over the grid, in the same frame (the piano roll's notes). The context is in CSS
     * pixels; the frame is the view actually drawn. Call `invalidate` when what it draws has changed.
     */
    drawBody?: (ctx: CanvasRenderingContext2D, frame: BodyFrame) => void
  } = $props()

  const WHEEL_ZOOM_RATE = 0.002
  const KEY_ZOOM = 1.25
  const LINE_PX = 16

  let root: HTMLDivElement
  let canvas: HTMLCanvasElement
  let width = $state(0)
  let height = $state(0)
  /** The pointer's x in the grid, `null` when it is outside. */
  let pointerX = $state<number | null>(null)
  let colors: Colors | null = null
  let font = ''
  let pending = 0
  let drawCount = 0
  /** Bumped when the surface must be drawn again though no number changed: a resize to the same size, a new pixel ratio. */
  let epoch = $state(0)

  const totalTicks = $derived(totalTicksOf(totalBars, signature))
  const clamped = $derived(clampView(view, width, totalTicks))

  /** The pointer's tick, snapped to the setting at the current zoom. */
  const snapped = (x: number): number => snapTick(xToTick(x, clamped), snapStep(snap, signature, clamped.pxPerTick))
  // The tick under a still pointer changes when the grid scrolls or zooms beneath it, or the snap changes.
  const hover = $derived(pointerX === null ? null : snapped(pointerX))
  $effect(() => onhover?.(hover))
  const localX = (event: { clientX: number }) => event.clientX - root.getBoundingClientRect().left

  /** Draws again on the next frame: for a host whose `drawBody` depends on state the grid cannot see. */
  export function invalidate() {
    if (!pending) pending = requestAnimationFrame(draw)
  }

  function set(next: View) {
    view = clampView(next, width, totalTicks)
  }

  function draw() {
    pending = 0
    const ctx = canvas.getContext('2d')
    if (!ctx || width === 0 || height === 0) return
    colors ??= readColors(root)
    const dpr = window.devicePixelRatio || 1
    // The bitmap follows the size and the pixel ratio; assigning a size clears it, so only when it differs.
    if (canvas.width !== Math.round(width * dpr)) canvas.width = Math.round(width * dpr)
    if (canvas.height !== Math.round(height * dpr)) canvas.height = Math.round(height * dpr)
    const start = performance.now()
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0)
    drawGrid(ctx, { width, height, view: clamped, signature, totalTicks, playhead, hover, colors, font }, dpr)
    // The cost of the last frame and the count of frames, for the frame-time measurement of the e2e spec.
    drawCount++
    root.dataset.drawMs = String(performance.now() - start)
    // The host's content is timed apart, so that the grid's own cost keeps its meaning.
    if (drawBody) {
      const bodyStart = performance.now()
      drawBody(ctx, { width, height, view: clamped, totalTicks, colors, dpr })
      root.dataset.bodyMs = String(performance.now() - bodyStart)
    }
    root.dataset.drawCount = String(drawCount)
  }

  // One frame per change: wheel bursts and playhead updates coalesce into the next animation frame.
  $effect(() => {
    void [clamped, signature, totalTicks, playhead, hover, width, height, epoch, drawBody]
    if (!pending) pending = requestAnimationFrame(draw)
  })

  $effect(() => {
    const observer = new ResizeObserver(() => {
      const box = root.getBoundingClientRect()
      width = Math.floor(box.width)
      height = Math.floor(box.height)
      epoch++
      // The theme or the fonts may have changed since the last size.
      colors = readColors(root)
      font = `500 12px ${getComputedStyle(root).getPropertyValue('--fk-font-mono').trim() || 'monospace'}`
    })
    observer.observe(root)
    return () => {
      observer.disconnect()
      cancelAnimationFrame(pending)
    }
  })

  // The pixel ratio changes with the monitor or the browser zoom, with no resize of the grid: draw again at the new ratio.
  $effect(() => {
    void epoch
    const query = matchMedia(`(resolution: ${window.devicePixelRatio || 1}dppx)`)
    const changed = () => epoch++
    query.addEventListener('change', changed, { once: true })
    return () => query.removeEventListener('change', changed)
  })

  // A wheel listener must be non-passive to keep the page from scrolling under a zoom.
  $effect(() => {
    const onWheel = (event: WheelEvent) => {
      const scale = event.deltaMode === 1 ? LINE_PX : event.deltaMode === 2 ? width : 1
      if (event.ctrlKey || event.metaKey) {
        event.preventDefault()
        set(zoomAt(clamped, Math.exp(-event.deltaY * scale * WHEEL_ZOOM_RATE), localX(event), width, totalTicks))
      } else if (event.shiftKey || Math.abs(event.deltaX) > Math.abs(event.deltaY)) {
        event.preventDefault()
        const delta = (event.shiftKey && event.deltaX === 0 ? event.deltaY : event.deltaX) * scale
        set(scrollBy(clamped, delta, width, totalTicks))
      }
    }
    root.addEventListener('wheel', onWheel, { passive: false })
    return () => root.removeEventListener('wheel', onWheel)
  })

  function onpointermove(event: PointerEvent) {
    pointerX = localX(event)
  }

  function onpointerleave() {
    pointerX = null
  }

  function onclick(event: MouseEvent) {
    if (event.clientY - root.getBoundingClientRect().top < RULER_HEIGHT) onseek?.(snapped(localX(event)))
  }

  function onkeydown(event: KeyboardEvent) {
    if (event.altKey || event.ctrlKey || event.metaKey) return
    const step = event.shiftKey ? width : width / 10
    if (event.key === 'ArrowRight') set(scrollBy(clamped, step, width, totalTicks))
    else if (event.key === 'ArrowLeft') set(scrollBy(clamped, -step, width, totalTicks))
    else if (event.key === '+' || event.key === '=') set(zoomAt(clamped, KEY_ZOOM, width / 2, width, totalTicks))
    else if (event.key === '-' || event.key === '_') set(zoomAt(clamped, 1 / KEY_ZOOM, width / 2, width, totalTicks))
    else if (event.key === 'Home') set({ ...clamped, scrollTick: 0 })
    else return
    event.preventDefault()
  }
</script>

<!-- A canvas has no children for assistive technology: the region is named and the keys are the way in; it is a custom widget, so the two a11y rules for plain elements do not apply. -->
<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
<div
  bind:this={root}
  class="timegrid"
  role="application"
  aria-label={label}
  tabindex="0"
  data-testid="timegrid"
  data-px-per-tick={clamped.pxPerTick}
  data-scroll-tick={clamped.scrollTick}
  {onpointermove}
  {onpointerleave}
  {onclick}
  {onkeydown}
>
  <canvas bind:this={canvas} style:width="{width}px" style:height="{height}px"></canvas>
</div>

<style>
  .timegrid {
    position: relative;
    width: 100%;
    height: 100%;
    min-height: 96px;
    overflow: hidden;
    cursor: crosshair;
    outline-offset: -2px;
  }
  canvas {
    position: absolute;
    inset: 0;
    display: block;
  }
</style>
