<script lang="ts">
  /**
   * A waveform drawn from its peaks (AU-2, `lib/audio/peaks/`): a canvas the size of its box, drawn once
   * per change on the next animation frame. It shows `frames` frames of the media from `from` (the whole
   * media by default), or, given `framesPerPixel`, as many as its width holds at that zoom (a surface
   * that scrolls). Without peaks yet (`null`: being read or made), a faint line holds its place. The
   * drawing itself (`drawWaveform`) takes any canvas, for a surface that draws many clips at once.
   */
  import { onMount } from 'svelte'
  import { drawWaveform } from '../audio/peaks/draw'
  import { readColors, trackInk } from './timegrid/render'
  import { theme } from '../theme.svelte'
  import type { Peaks } from '../audio/peaks/format'

  interface Props {
    peaks: Peaks | null
    /** The frame at the left edge. */
    from?: number
    /** The frames the width shows; the rest of the media from `from` when not given. */
    frames?: number
    /** Frames a CSS pixel holds, instead of `frames`: the zoom of a surface that scrolls. */
    framesPerPixel?: number
    /** Any CSS colour; the text colour by default. */
    color?: string
    /** One lane per channel, the channels merged, or `auto`: lanes when each has room. */
    lanes?: boolean | 'auto'
    /** `color` is a track's hue: drawn with the theme's ink like the other lines of a track (#687). */
    track?: boolean
  }

  let { peaks, from = 0, frames, framesPerPixel, color = 'currentColor', lanes = 'auto', track = false }: Props = $props()

  let root: HTMLElement
  let canvas: HTMLCanvasElement
  let width = $state(0)
  let height = $state(0)
  let dpr = $state(1)
  let pending = 0
  let drawCount = 0
  /** The colour the canvas takes, resolved from `color` (a variable, `currentColor`). */
  let resolved = ''

  const perPixel = $derived.by(() => {
    if (framesPerPixel !== undefined) return framesPerPixel
    if (!peaks || width === 0) return 0
    return (frames ?? peaks.frames - from) / width
  })

  function draw() {
    pending = 0
    const ctx = canvas?.getContext('2d')
    if (!ctx || width === 0 || height === 0) return
    if (canvas.width !== Math.round(width * dpr)) canvas.width = Math.round(width * dpr)
    if (canvas.height !== Math.round(height * dpr)) canvas.height = Math.round(height * dpr)
    const start = performance.now()
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0)
    ctx.clearRect(0, 0, width, height)
    if (!resolved) {
      const computed = getComputedStyle(root).color
      resolved = track ? trackInk(computed, readColors(root)) : computed
    }
    let level = ''
    if (peaks) {
      const report = drawWaveform(ctx, peaks, { x: 0, y: 0, width, height }, { from, framesPerPixel: perPixel }, { color: resolved, lanes, dpr })
      level = report ? String(report.level) : ''
    } else {
      // A placeholder while the peaks are read or made.
      ctx.globalAlpha = 0.35
      ctx.fillStyle = resolved
      ctx.fillRect(0, Math.floor(height / 2), width, 1)
      ctx.globalAlpha = 1
    }
    // The cost of the last frame, the level read and the count of frames, for the browser checks.
    drawCount++
    root.dataset.drawMs = String(performance.now() - start)
    root.dataset.level = level
    root.dataset.drawCount = String(drawCount)
  }

  const schedule = () => {
    if (!pending && typeof requestAnimationFrame !== 'undefined') pending = requestAnimationFrame(draw)
  }

  // A new colour or a new theme (#687): the colour is resolved again from the tokens, and the waveform drawn again.
  const page = theme()
  $effect(() => {
    void [color, track, page.resolved]
    resolved = ''
    schedule()
  })

  $effect(() => {
    void [peaks, from, perPixel, lanes, width, height, dpr]
    schedule()
  })

  onMount(() => {
    const observer = new ResizeObserver(([entry]) => {
      if (!entry) return
      width = entry.contentRect.width
      height = entry.contentRect.height
      dpr = window.devicePixelRatio || 1
    })
    observer.observe(root)
    return () => {
      observer.disconnect()
      if (pending) cancelAnimationFrame(pending)
    }
  })
</script>

<div class="waveform" bind:this={root} style:color data-testid="waveform" data-state={peaks ? 'ready' : 'empty'} aria-hidden="true">
  <canvas bind:this={canvas}></canvas>
</div>

<style>
  .waveform {
    position: relative;
    width: 100%;
    height: 100%;
    min-width: 0;
    overflow: hidden;
  }
  canvas {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
  }
</style>
