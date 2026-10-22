<script lang="ts">
  /**
   * A canvas that fits its box and calls `draw` with a context in CSS pixels. `draw` is called inside
   * an effect: whatever reactive state it reads makes the surface draw again, once per change burst.
   */
  import { readColors, type Colors } from '../../lib/components/timegrid/render'

  let {
    draw,
    label,
    testid,
  }: {
    draw: (ctx: CanvasRenderingContext2D, width: number, height: number, colors: Colors, font: string) => void
    label: string
    testid?: string
  } = $props()

  let root: HTMLDivElement
  let canvas: HTMLCanvasElement
  let width = $state(0)
  let height = $state(0)
  let epoch = $state(0)
  let colors: Colors | null = null
  let font = ''
  let ratio = $state(1)

  $effect(() => {
    const observer = new ResizeObserver(() => {
      const box = root.getBoundingClientRect()
      width = Math.floor(box.width)
      height = Math.floor(box.height)
      ratio = window.devicePixelRatio || 1
      colors = readColors(root)
      font = `500 11px ${getComputedStyle(root).getPropertyValue('--fk-font-mono').trim() || 'monospace'}`
      epoch++
    })
    observer.observe(root)
    return () => observer.disconnect()
  })

  $effect(() => {
    void epoch
    const ctx = canvas.getContext('2d')
    if (!ctx || width === 0 || height === 0 || !colors) return
    if (canvas.width !== Math.round(width * ratio)) canvas.width = Math.round(width * ratio)
    if (canvas.height !== Math.round(height * ratio)) canvas.height = Math.round(height * ratio)
    ctx.setTransform(ratio, 0, 0, ratio, 0, 0)
    const start = performance.now()
    draw(ctx, width, height, colors, font)
    root.dataset.drawMs = String(performance.now() - start)
  })
</script>

<div bind:this={root} class="surface" role="img" aria-label={label} data-testid={testid}>
  <canvas bind:this={canvas} style:width="{width}px" style:height="{height}px"></canvas>
</div>

<style>
  .surface {
    position: relative;
    width: 100%;
    height: 100%;
    overflow: hidden;
  }
  canvas {
    position: absolute;
    inset: 0;
    display: block;
  }
</style>
