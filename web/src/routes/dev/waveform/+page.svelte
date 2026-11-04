<script lang="ts">
  // A test surface for the waveform (AU-2), until the arrangement draws audio placements (AU-6): one
  // hour of stereo at 48 kHz, its peaks made up here (a slow swell with a beat in it, never above 0.9), scrolled with the
  // wheel and zoomed with Ctrl and the wheel. It shows only on the dev server and in the e2e build
  // (`VITE_FAULK_DEV_ROUTES=1`): a production build renders an empty page. Plain English text, not part
  // of the product UI.
  import '../../../lib/styles/app.css'
  import Waveform from '../../../lib/components/Waveform.svelte'
  import { TRACK_COLORS } from '../../../lib/project/tracks'
  import { BLOCK, FACTOR, LEVELS, type Peaks } from '../../../lib/audio/peaks/format'

  const enabled = import.meta.env.DEV || import.meta.env.VITE_FAULK_DEV_ROUTES === '1'

  const RATE = 48_000
  const FRAMES = RATE * 3600

  /** One hour of made-up peaks: level 0 by formula, the coarser levels reduced from it as crates/audio-import does. */
  function hour(): Peaks {
    const channels = 2
    const count0 = Math.ceil(FRAMES / BLOCK)
    const level0 = new Int8Array(count0 * 2 * channels)
    for (let i = 0; i < count0; i++) {
      const t = (i * BLOCK) / RATE
      const swell = 0.25 + 0.6 * Math.abs(Math.sin(t / 37))
      const beat = (t * 2) % 1 < 0.08 ? 0.3 : 0
      for (let c = 0; c < channels; c++) {
        const a = Math.min(0.9, swell * (c === 0 ? 1 : 0.7) + beat)
        level0[(i * channels + c) * 2] = Math.floor(-a * 127)
        level0[(i * channels + c) * 2 + 1] = Math.ceil(a * 127)
      }
    }
    const levels = [{ block: BLOCK, count: count0, data: level0 }]
    for (let level = 1; level < LEVELS; level++) {
      const below = levels[level - 1]!
      const count = Math.ceil(FRAMES / (BLOCK * FACTOR ** level))
      const data = new Int8Array(count * 2 * channels)
      for (let i = 0; i < count; i++) {
        for (let c = 0; c < channels; c++) {
          let lo = 127
          let hi = -127
          for (let j = i * FACTOR; j < Math.min(below.count, (i + 1) * FACTOR); j++) {
            lo = Math.min(lo, below.data[(j * channels + c) * 2]!)
            hi = Math.max(hi, below.data[(j * channels + c) * 2 + 1]!)
          }
          data[(i * channels + c) * 2] = lo
          data[(i * channels + c) * 2 + 1] = hi
        }
      }
      levels.push({ block: BLOCK * FACTOR ** level, count, data })
    }
    return { sampleRate: RATE, channels, frames: FRAMES, levels }
  }

  const peaks = enabled ? hour() : null
  let box = $state<HTMLElement>()
  let from = $state(0)
  let framesPerPixel = $state(16_384)

  function onWheel(event: WheelEvent) {
    event.preventDefault()
    const width = box?.clientWidth ?? 1
    const most = FRAMES / width
    if (event.ctrlKey) {
      // Zoom about the pointer: the frame under it stays under it.
      const x = event.clientX - (box?.getBoundingClientRect().left ?? 0)
      const at = from + x * framesPerPixel
      framesPerPixel = Math.min(most, Math.max(1, framesPerPixel * 2 ** (event.deltaY / 120)))
      from = at - x * framesPerPixel
    } else {
      from += (event.shiftKey ? event.deltaY || event.deltaX : event.deltaY) * framesPerPixel
    }
    from = Math.min(Math.max(0, from), Math.max(0, FRAMES - width * framesPerPixel))
  }

  // Non-passive, to keep the browser from zooming the page under a Ctrl + wheel.
  $effect(() => {
    const target = box
    if (!target) return
    target.addEventListener('wheel', onWheel, { passive: false })
    return () => target.removeEventListener('wheel', onWheel)
  })
</script>

<svelte:head>
  <title>Waveform (test surface)</title>
</svelte:head>

{#if enabled}
  <!-- The page's theme (#687): the waveform is a track's hue, drawn with the theme's ink like a clip in the Clips panel. -->
  <main class="demo">
    <header>
      <span>One hour, stereo, 48 kHz. Wheel: scroll. Ctrl + wheel: zoom.</span>
    </header>
    <div class="wave" bind:this={box} data-testid="waveform-surface" data-from={from} data-frames-per-pixel={framesPerPixel} data-frames={FRAMES}>
      <Waveform {peaks} {from} {framesPerPixel} color={TRACK_COLORS[7]} track />
    </div>
  </main>
{/if}

<style>
  .demo {
    display: grid;
    grid-template-rows: auto 1fr;
    height: 100vh;
    background: var(--fk-bg);
    color: var(--fk-text);
  }
  header {
    display: flex;
    gap: var(--fk-space-4);
    align-items: center;
    padding: var(--fk-space-2) var(--fk-space-4);
  }
  .wave {
    min-height: 0;
    margin: var(--fk-space-4);
  }
</style>
