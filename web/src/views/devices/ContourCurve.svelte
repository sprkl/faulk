<script lang="ts">
  /**
   * The response curve of an equaliser Contour (#156), on top of its generated controls: the
   * spectrum of its output behind (the engine's report, computed off the audio path), the curve of
   * its bands that are on, and one handle per band. Every gesture is a `device.set_param`: a drag
   * moves the frequency (left and right) and the gain (up and down; a cut band has none), one
   * history group; the wheel over a handle changes its Q, one group per wheel gesture; a double
   * click switches the band; the arrows on a focused handle move it (Shift: the Q), Enter switches it.
   */
  import { onDestroy } from 'svelte'
  import { BANDS, FREQ_MAX, FREQ_MIN, GAIN_MAX, Q_MAX, Q_MIN, curveHz, hzOfX, isCut, isOn, responseDb, xOfHz, type Band } from '../../lib/contour/response'
  import type { Shown } from '../../lib/contour/spectrum'
  import { deviceText, formatValue, type DeviceDescriptor } from '../../lib/devices'
  import { useStores } from '../../lib/stores'

  let {
    descriptor,
    name,
    valueOf,
    spectrum,
    sampleRate,
    bypassed = false,
    onchange,
    onbegin,
    onend,
  }: {
    descriptor: DeviceDescriptor
    name: string
    /** The value of a parameter: the project's, an automation lane's, else its default. */
    valueOf: (key: string) => number
    /** The last spectrum of the device's output, `null` when none is fresh. */
    spectrum: Shown | null
    /** The rate the curve's bilinear warp follows: the engine's, else the device's last spectrum's (`curveRate`). */
    sampleRate: number
    bypassed?: boolean
    onchange: (key: string, value: number) => void
    onbegin: () => void
    onend: () => void
  } = $props()

  const { ui } = useStores()
  const t = ui.t

  const HEIGHT = 150
  /** The curve's scale, in dB from the middle line. */
  const RANGE = GAIN_MAX
  /** The spectrum's scale: 0 dBFS at the top, this at the bottom. */
  const SPECTRUM_FLOOR = -96
  /** Steps of the keyboard: a sixth of an octave, half a decibel, a tenth of the Q. */
  const FREQ_STEP = Math.pow(2, 1 / 6)
  const GAIN_STEP = 0.5
  const Q_STEP = 1.1
  /** A wheel gesture ends once the wheel rests this long. */
  const WHEEL_REST_MS = 400

  let width = $state(480)
  const w = $derived(Math.max(240, width))
  let svg = $state<SVGSVGElement>()

  const xOf = (hz: number) => xOfHz(hz) * w
  const yOfDb = (db: number) => HEIGHT / 2 - (Math.max(-RANGE, Math.min(RANGE, db)) / RANGE) * (HEIGHT / 2 - 8)
  const dbOfY = (y: number) => ((HEIGHT / 2 - y) / (HEIGHT / 2 - 8)) * RANGE
  const yOfLevel = (db: number) => (Math.min(0, Math.max(SPECTRUM_FLOOR, db)) / SPECTRUM_FLOOR) * HEIGHT

  const curve = $derived.by(() => {
    const points = curveHz(Math.max(64, Math.round(w / 3)))
    return points.map((hz, i) => `${i === 0 ? 'M' : 'L'}${xOf(hz).toFixed(1)} ${yOfDb(responseDb(valueOf, hz, sampleRate)).toFixed(1)}`).join(' ')
  })

  // The spectrum, held a little so it does not flicker: up at once, down by a share of the gap.
  let held: Float32Array | null = null
  let lastBands: Float32Array | null = null
  const spectrumPath = $derived.by(() => {
    const s = spectrum
    if (!s) {
      held = null
      lastBands = null
      return ''
    }
    let h = held
    if (s.bands !== lastBands) {
      lastBands = s.bands
      if (!h || h.length !== s.bands.length) h = Float32Array.from(s.bands)
      else for (let i = 0; i < h.length; i++) h[i] = s.bands[i] > h[i] ? s.bands[i] : h[i] + (s.bands[i] - h[i]) * 0.35
      held = h
    }
    if (!h) return ''
    const n = h.length
    const nyquist = s.sampleRate / 2
    let d = `M0 ${HEIGHT}`
    for (let i = 0; i < n; i++) {
      // Band i spans 20 * 1000^(i/n) to 20 * 1000^((i+1)/n): its centre on the axis is (i + 0.5) / n.
      const centre = FREQ_MIN * Math.pow(FREQ_MAX / FREQ_MIN, (i + 0.5) / n)
      if (centre >= nyquist) break
      d += ` L${(((i + 0.5) / n) * w).toFixed(1)} ${yOfLevel(h[i]).toFixed(1)}`
    }
    return `${d} L${w} ${HEIGHT} Z`
  })

  const label = (band: Band) => deviceText(ui.lang, `contour.${band.key}_on`) ?? band.key
  const param = (key: string) => descriptor.params.find((p) => p.key === key)
  const text = (key: string) => {
    const p = param(key)
    return p ? formatValue(p, valueOf(key)) : ''
  }
  const handleLabel = (band: Band) => {
    const parts = [label(band), text(`${band.key}_freq`)]
    if (!isCut(band.shape)) parts.push(text(`${band.key}_gain`))
    parts.push(t('contour.qValue', { value: valueOf(`${band.key}_q`).toFixed(2) }))
    if (!isOn(band, valueOf)) parts.push(t('contour.bandOff'))
    return parts.join(', ')
  }
  const handleY = (band: Band) => yOfDb(isCut(band.shape) ? 0 : valueOf(`${band.key}_gain`))

  const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v))
  /** A value set only when it changes (a drag that does not move sends nothing). */
  function set(key: string, value: number) {
    if (Math.abs(valueOf(key) - value) > 1e-9) onchange(key, value)
  }

  // ---- drag: the handle follows the pointer's movement from where it was grabbed, once past a few pixels (a click or a
  // double click does not nudge the band).
  /** Pixels the pointer moves before a drag changes anything. */
  const DEAD_ZONE = 3
  let dragging: { band: Band; x: number; y: number; freqX: number; gainY: number; moved: boolean } | null = null
  function pointerDown(event: PointerEvent, band: Band) {
    if (event.button !== 0) return
    event.preventDefault()
    ;(event.currentTarget as Element).setPointerCapture?.(event.pointerId)
    dragging = {
      band,
      x: event.clientX,
      y: event.clientY,
      freqX: xOfHz(valueOf(`${band.key}_freq`)),
      gainY: yOfDb(isCut(band.shape) ? 0 : valueOf(`${band.key}_gain`)),
      moved: false,
    }
    // A wheel or arrows group still open ends first: the drag is a group of its own.
    endWheel()
    keyUp()
    onbegin()
  }
  function pointerMove(event: PointerEvent) {
    if (!dragging || !svg) return
    const box = svg.getBoundingClientRect()
    const dx = event.clientX - dragging.x
    const dy = event.clientY - dragging.y
    if (!dragging.moved && Math.hypot(dx, dy) < DEAD_ZONE) return
    dragging.moved = true
    const band = dragging.band
    const x = dragging.freqX + dx / Math.max(1, box.width)
    const y = dragging.gainY + (dy / Math.max(1, box.height)) * HEIGHT
    set(`${band.key}_freq`, Math.round(clamp(hzOfX(x), FREQ_MIN, FREQ_MAX) * 10) / 10)
    if (!isCut(band.shape)) set(`${band.key}_gain`, Math.round(clamp(dbOfY(y), -GAIN_MAX, GAIN_MAX) * 10) / 10)
  }
  function pointerUp() {
    if (!dragging) return
    dragging = null
    onend()
  }

  // ---- wheel: the Q, one group per gesture
  let wheelTimer: ReturnType<typeof setTimeout> | null = null
  function endWheel() {
    if (wheelTimer === null) return
    clearTimeout(wheelTimer)
    wheelTimer = null
    onend()
  }
  function wheel(event: WheelEvent, band: Band) {
    event.preventDefault()
    // A held arrow's group ends first: the wheel's rest must not close it under the key.
    keyUp()
    // Every tick: a click elsewhere may have closed the group meanwhile (a release ends every gesture); `onbegin` opens
    // one only when none is open.
    onbegin()
    if (wheelTimer !== null) clearTimeout(wheelTimer)
    wheelTimer = setTimeout(endWheel, WHEEL_REST_MS)
    const q = valueOf(`${band.key}_q`) * Math.pow(2, -event.deltaY / 400)
    set(`${band.key}_q`, Math.round(clamp(q, Q_MIN, Q_MAX) * 1000) / 1000)
  }
  // A wheel listener that may prevent the page's scroll (Svelte's are passive).
  function wheelOn(node: SVGElement, band: Band) {
    const listener = (event: WheelEvent) => wheel(event, band)
    node.addEventListener('wheel', listener, { passive: false })
    return { destroy: () => node.removeEventListener('wheel', listener) }
  }
  onDestroy(() => {
    endWheel()
    pointerUp()
  })

  const toggle = (band: Band) => onchange(`${band.key}_on`, isOn(band, valueOf) ? 0 : 1)

  /** A held arrow (its key repeats) is one history group, closed when the key comes up or the point loses focus. */
  let keyHeld = false
  function keyUp() {
    if (!keyHeld) return
    keyHeld = false
    onend()
  }

  function key(event: KeyboardEvent, band: Band) {
    const freqKey = `${band.key}_freq`
    const gainKey = `${band.key}_gain`
    const qKey = `${band.key}_q`
    const freq = valueOf(freqKey)
    switch (event.key) {
      case 'ArrowLeft':
        set(freqKey, Math.round(clamp(freq / FREQ_STEP, FREQ_MIN, FREQ_MAX) * 10) / 10)
        break
      case 'ArrowRight':
        set(freqKey, Math.round(clamp(freq * FREQ_STEP, FREQ_MIN, FREQ_MAX) * 10) / 10)
        break
      case 'ArrowUp':
      case 'ArrowDown': {
        const up = event.key === 'ArrowUp'
        if (event.shiftKey || isCut(band.shape)) set(qKey, Math.round(clamp(valueOf(qKey) * (up ? Q_STEP : 1 / Q_STEP), Q_MIN, Q_MAX) * 1000) / 1000)
        else set(gainKey, clamp(valueOf(gainKey) + (up ? GAIN_STEP : -GAIN_STEP), -GAIN_MAX, GAIN_MAX))
        break
      }
      case 'Enter':
        toggle(band)
        event.preventDefault()
        return
      default:
        return
    }
    event.preventDefault()
  }

  /** The arrows open their group before the first change (the card's gesture opens one only when none is open). */
  function keyDown(event: KeyboardEvent, band: Band) {
    if (event.key.startsWith('Arrow') && !keyHeld) {
      // A wheel group waiting for its rest ends first: its timer must not close the arrows' group.
      endWheel()
      keyHeld = true
      onbegin()
    }
    key(event, band)
  }

  const GRID_HZ = [50, 100, 200, 500, 1000, 2000, 5000, 10000]
  const GRID_LABELS: Record<number, string> = { 100: '100', 1000: '1k', 10000: '10k' }
</script>

<div class="contour" class:bypassed bind:clientWidth={width} data-testid="contour-curve" data-spectrum={spectrum ? 'on' : 'off'}>
  <svg
    bind:this={svg}
    width={w}
    height={HEIGHT}
    viewBox="0 0 {w} {HEIGHT}"
    role="group"
    aria-label={t('contour.curve', { name })}
    onpointermove={pointerMove}
    onpointerup={pointerUp}
    onpointercancel={pointerUp}
  >
    <title>{t('contour.curveHint')}</title>
    <g class="grid" aria-hidden="true">
      {#each GRID_HZ as hz (hz)}
        <line x1={xOf(hz)} x2={xOf(hz)} y1="0" y2={HEIGHT} />
        {#if GRID_LABELS[hz]}<text x={xOf(hz) + 3} y={HEIGHT - 4}>{GRID_LABELS[hz]}</text>{/if}
      {/each}
      {#each [-12, 12] as db (db)}<line class="minor" x1="0" x2={w} y1={yOfDb(db)} y2={yOfDb(db)} />{/each}
      <line class="zero" x1="0" x2={w} y1={yOfDb(0)} y2={yOfDb(0)} />
    </g>
    {#if spectrumPath}<path class="spectrum" d={spectrumPath} data-testid="contour-spectrum" aria-hidden="true" />{/if}
    <path class="curve" d={curve} data-testid="contour-response" aria-hidden="true" />
    {#each BANDS as band, i (band.key)}
      <g
        class="handle"
        class:off={!isOn(band, valueOf)}
        role="button"
        tabindex="0"
        aria-label={handleLabel(band)}
        aria-pressed={isOn(band, valueOf)}
        data-testid="contour-handle"
        data-band={band.key}
        transform="translate({xOf(valueOf(`${band.key}_freq`)).toFixed(1)} {handleY(band).toFixed(1)})"
        onpointerdown={(event) => pointerDown(event, band)}
        ondblclick={() => toggle(band)}
        onkeydown={(event) => keyDown(event, band)}
        onkeyup={keyUp}
        onblur={keyUp}
        use:wheelOn={band}
      >
        <circle r="9" />
        <text text-anchor="middle" dy="3.5">{i + 1}</text>
      </g>
    {/each}
  </svg>
</div>

<style>
  .contour {
    width: 100%;
    min-width: 240px;
    max-width: 640px;
    border-radius: var(--fk-radius-work);
    background: var(--fk-surface);
    overflow: hidden;
    touch-action: none;
  }
  .contour.bypassed {
    opacity: 0.45;
  }
  svg {
    display: block;
  }
  .grid line {
    stroke: var(--fk-line);
    stroke-width: 1;
  }
  .grid line.minor {
    stroke-dasharray: 2 3;
  }
  .grid line.zero {
    stroke: var(--fk-text-muted);
    stroke-opacity: 0.5;
  }
  .grid text {
    fill: var(--fk-text-muted);
    font-size: 10px;
    font-family: var(--fk-font-mono);
  }
  .spectrum {
    fill: var(--fk-text-muted);
    fill-opacity: 0.18;
    stroke: var(--fk-text-muted);
    stroke-opacity: 0.45;
    stroke-width: 1;
  }
  .curve {
    fill: none;
    stroke: var(--fk-accent);
    stroke-width: 2;
  }
  .handle {
    cursor: grab;
    outline: none;
  }
  .handle circle {
    fill: var(--fk-accent);
    stroke: var(--fk-bg);
    stroke-width: 1.5;
  }
  .handle text {
    fill: var(--fk-on-accent);
    font-size: 10px;
    font-weight: 600;
    pointer-events: none;
    user-select: none;
  }
  .handle.off circle {
    fill: var(--fk-bg);
    stroke: var(--fk-text-muted);
  }
  .handle.off text {
    fill: var(--fk-text-muted);
  }
  .handle:focus-visible circle {
    stroke: var(--fk-text);
    stroke-width: 2.5;
  }
</style>
