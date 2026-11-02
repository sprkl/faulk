/**
 * The visualiser of a playing track, over its cover: the cover's colour foci swell with their
 * frequency band and drift like the landing page's hero glow, and the waveform runs across as a
 * line of light points. Loaded on the first play only, never under `prefers-reduced-motion`.
 *
 * The audio goes through the page's `AudioContext` (a `MediaElementAudioSourceNode` per track, made once:
 * an element can only have one), then an `AnalyserNode`, then the speakers. The audio elements
 * are `crossorigin="anonymous"` and the CDN answers with open CORS, so the analyser hears the
 * signal (without CORS the element would not load at all, rather than play silence).
 */

import { bandLevels, follow } from './bands'

/** The cover's coordinate space (`ART_SIZE` of `art.ts`). */
const SIZE = 400
const INK = '#131211'

interface Focus {
  x: number
  y: number
  r: number
  c: string
}

const graphs = new WeakMap<HTMLMediaElement, AnalyserNode>()

function analyserFor(audio: HTMLMediaElement, context: AudioContext): AnalyserNode {
  let analyser = graphs.get(audio)
  if (!analyser) {
    const source = context.createMediaElementSource(audio)
    analyser = context.createAnalyser()
    analyser.fftSize = 2048
    analyser.smoothingTimeConstant = 0.72
    source.connect(analyser)
    analyser.connect(context.destination)
    graphs.set(audio, analyser)
  }
  return analyser
}

function rgba(hex: string, a: number): string {
  const n = parseInt(hex.slice(1), 16)
  return `rgba(${(n >> 16) & 255},${(n >> 8) & 255},${n & 255},${Math.max(0, Math.min(1, a)).toFixed(3)})`
}

export interface Viz {
  start(): void
  stop(): void
  destroy(): void
}

/**
 * Builds the visualiser of `stage` (an element with `data-foci`, the cover's foci as JSON) for
 * `audio`, on a running `context`. Throws if the browser cannot (no 2D canvas): the caller then
 * plays without it, and the audio is not rerouted.
 */
export function createViz(stage: HTMLElement, audio: HTMLMediaElement, context: AudioContext): Viz {
  const foci = JSON.parse(stage.dataset.foci ?? '[]') as Focus[]
  const canvas = document.createElement('canvas')
  canvas.className = 'fk-live-canvas'
  canvas.setAttribute('aria-hidden', 'true')
  const ctx = canvas.getContext('2d')
  if (!ctx) throw new Error('no 2D canvas')
  const analyser = analyserFor(audio, context)
  stage.append(canvas)

  const spectrum = new Uint8Array(analyser.frequencyBinCount)
  const wave = new Float32Array(analyser.fftSize)
  const levels = new Float32Array(Math.max(1, foci.length))
  const energy = new Float32Array(levels.length)
  let w = 1
  let h = 1
  let raf = 0
  let running = false
  let last = 0
  let t = 0
  let peak = 0.2

  const resize = () => {
    const scale = Math.min(2, Math.max(1, devicePixelRatio || 1))
    w = Math.max(1, Math.round(stage.clientWidth * scale))
    h = Math.max(1, Math.round(stage.clientHeight * scale))
    if (canvas.width !== w || canvas.height !== h) {
      canvas.width = w
      canvas.height = h
    }
  }
  const ro = new ResizeObserver(resize)
  ro.observe(stage)
  resize()

  const draw = (now: number) => {
    if (!running) return
    const dt = last ? Math.min(0.05, (now - last) / 1000) : 0
    last = now
    t += dt
    analyser.getByteFrequencyData(spectrum)
    analyser.getFloatTimeDomainData(wave)
    bandLevels(spectrum, levels)
    for (let i = 0; i < energy.length; i++) energy[i] = follow(energy[i], levels[i] * 1.35, dt)

    const k = w / SIZE
    ctx.globalCompositeOperation = 'source-over'
    ctx.fillStyle = INK
    ctx.fillRect(0, 0, w, h)
    // The foci: the cover's glow, each breathing with its band and wandering slowly.
    ctx.globalCompositeOperation = 'screen'
    foci.forEach((f, i) => {
      const e = energy[i] ?? 0
      const x = (f.x + Math.sin(t * 0.21 + i * 2.1) * 14) * k
      const y = (f.y + Math.cos(t * 0.17 + i * 1.3) * 12) * k
      const r = f.r * (0.8 + e * 0.6) * k
      const g = ctx.createRadialGradient(x, y, 0, x, y, r)
      g.addColorStop(0, rgba(f.c, 0.5 + e * 0.5))
      g.addColorStop(1, rgba(f.c, 0))
      ctx.fillStyle = g
      ctx.fillRect(x - r, y - r, r * 2, r * 2)
    })
    // The waveform: a line of points, brighter and lilac where the signal is strong.
    ctx.globalCompositeOperation = 'source-over'
    const points = 180
    const mid = h * 0.47
    const stride = Math.floor(wave.length / points)
    const dot = Math.max(1.5, w / 300)
    // Auto-gain: a quiet passage still draws a readable line; the peak follows like a meter.
    let top = 0
    for (let p = 0; p < points; p++) top = Math.max(top, Math.abs(wave[p * stride] || 0))
    peak = Math.max(0.04, follow(peak, top, dt))
    const gain = 1 / Math.max(peak, 0.15)
    for (let p = 0; p < points; p++) {
      const s = Math.max(-1, Math.min(1, (wave[p * stride] || 0) * gain))
      const a = Math.min(1, Math.abs(s) * 2.4)
      ctx.fillStyle = a > 0.5 ? `rgba(156,146,255,${(0.55 + a * 0.45).toFixed(3)})` : `rgba(236,233,230,${(0.5 + a * 0.5).toFixed(3)})`
      ctx.fillRect(((p + 0.5) / points) * w - dot / 2, mid - s * h * 0.3 - dot / 2, dot, dot)
    }
    raf = requestAnimationFrame(draw)
  }

  return {
    start() {
      if (running) return
      if (context.state === 'suspended') void context.resume()
      running = true
      last = 0
      stage.classList.add('is-live')
      raf = requestAnimationFrame(draw)
    },
    stop() {
      running = false
      cancelAnimationFrame(raf)
      stage.classList.remove('is-live')
    },
    destroy() {
      this.stop()
      ro.disconnect()
      canvas.remove()
    },
  }
}
