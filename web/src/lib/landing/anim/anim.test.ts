import { readdirSync, readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'
import { DropRing, HitClock, MIN_HIT_INTERVAL } from './clocks'
import { createFocus, packFoci, stepFoci } from './foci'
import { canvasSize, frameDelta, gateState, renderScale } from './gate'
import { seekRequest } from './index'
import { homeOf, lcg, particleConstants, stepParticle, waveform } from './particles'
import { approach, idlePoint } from './pointer'
import { SIM } from './shaders/hero'

const dir = new URL('./', import.meta.url)

describe('anim module', () => {
  it('imports nothing outside its folder besides ogl (the chunk hash covers its sources and ogl)', () => {
    const files = readdirSync(dir, { recursive: true, encoding: 'utf8' }).filter((f) => /\.ts$/.test(f) && !f.endsWith('.test.ts'))
    expect(files.length).toBeGreaterThan(8)
    for (const file of files) {
      const source = readFileSync(new URL(file, dir), 'utf8')
      const imports = [...source.matchAll(/from '([^']+)'/g)].map((m) => m[1])
      for (const spec of imports) expect(spec === 'ogl' || spec.startsWith('./') || spec.startsWith('../shaders/'), `${file}: ${spec}`).toBe(true)
      for (const spec of imports) expect(spec.startsWith('../') && !spec.startsWith('../shaders/'), `${file}: ${spec}`).toBe(false)
    }
  })
})

describe('gate', () => {
  const base = { hidden: false, reducedMotion: false, visibleStages: 1, contextLost: false }

  it('runs only with a visible stage in a visible tab', () => {
    expect(gateState(base)).toBe('run')
    expect(gateState({ ...base, hidden: true })).toBe('idle')
    expect(gateState({ ...base, visibleStages: 0 })).toBe('idle')
  })

  it('shows one still frame under reduced motion, and nothing once the context is lost', () => {
    expect(gateState({ ...base, reducedMotion: true })).toBe('still')
    expect(gateState({ ...base, reducedMotion: true, hidden: true })).toBe('idle')
    expect(gateState({ ...base, contextLost: true })).toBe('off')
    expect(gateState({ ...base, contextLost: true, reducedMotion: true, hidden: true })).toBe('off')
  })

  it('caps the render scale at 2 and tolerates bad ratios', () => {
    expect(renderScale(1)).toBe(1)
    expect(renderScale(1.5)).toBe(1.5)
    expect(renderScale(3)).toBe(2)
    expect(renderScale(0)).toBe(1)
    expect(renderScale(NaN)).toBe(1)
  })

  it('sizes canvases in device px, never 0, never above the cap', () => {
    expect(canvasSize(300, 150, 2)).toEqual({ w: 600, h: 300 })
    expect(canvasSize(0, 0, 2)).toEqual({ w: 1, h: 1 })
    expect(canvasSize(5000, 10, 2)).toEqual({ w: 4096, h: 20 })
  })

  it('clamps the frame delta so a pause never explodes the springs', () => {
    expect(frameDelta(1016.7, 1000)).toBeCloseTo(0.0167)
    expect(frameDelta(5000, 1000)).toBe(0.05)
    expect(frameDelta(900, 1000)).toBe(0)
    expect(frameDelta(NaN, 1000)).toBe(0)
  })
})

describe('HitClock', () => {
  it('hits on its own once per period, from its phase', () => {
    const clock = new HitClock(1.2, 0.3)
    const hits: number[] = []
    for (let t = 0; t <= 4; t += 1 / 60) if (clock.tick(t)) hits.push(t)
    expect(hits.length).toBe(4)
    for (const [i, expected] of [0.3, 1.5, 2.7, 3.9].entries()) expect(Math.abs(hits[i] - expected)).toBeLessThan(0.02)
  })

  it('refuses pointer hits closer than the WCAG-safe interval, and realigns the beat', () => {
    const clock = new HitClock(1.2, 0)
    expect(clock.tick(0)).toBe(true)
    expect(clock.hit(0.1)).toBe(false)
    expect(clock.hit(MIN_HIT_INTERVAL)).toBe(true)
    expect(clock.since(MIN_HIT_INTERVAL + 0.05)).toBeCloseTo(0.05)
    // The next autonomous hit is one period after the manual one.
    expect(clock.tick(1.2)).toBe(false)
    expect(clock.tick(MIN_HIT_INTERVAL + 1.2)).toBe(true)
  })

  it('never fires more than three times per second however it is driven', () => {
    const clock = new HitClock(0.1, 0)
    let hits = 0
    for (let t = 0; t < 1; t += 0.01) {
      if (clock.tick(t)) hits++
      if (clock.hit(t)) hits++
    }
    expect(hits).toBeLessThanOrEqual(3)
  })
})

describe('DropRing', () => {
  it('drops at the centre every period and overwrites the oldest slot', () => {
    const ring = new DropRing(3, 1)
    expect(ring.tick(0)).toBe(true)
    expect(ring.tick(0.5)).toBe(false)
    expect(ring.tick(1)).toBe(true)
    expect(ring.tick(2)).toBe(true)
    expect(ring.tick(3)).toBe(true)
    expect(ring.alive(3)).toBe(3)
    expect(ring.data.slice(0, 4)).toEqual([0.5, 0.5, 3, 1])
  })

  it('rate-limits pointer drops and ignores a slow pointer', () => {
    const ring = new DropRing(12, 0.9, 0.14)
    expect(ring.pointer(0, 0.2, 0.2, 0.01)).toBe(false)
    expect(ring.pointer(0, 0.2, 0.2, 1)).toBe(true)
    expect(ring.pointer(0.1, 0.3, 0.2, 1)).toBe(false)
    expect(ring.pointer(0.15, 0.3, 0.2, 1)).toBe(true)
    expect(ring.alive(0.15)).toBe(2)
    expect(ring.alive(10)).toBe(0)
  })
})

describe('foci', () => {
  it('rest at their position, wander a little, and pack as vec4s', () => {
    const foci = [createFocus(0.3, 0.6, 1, 0.5, 0.4)]
    for (let t = 0; t < 20; t += 1 / 60) stepFoci(foci, t, 1 / 60, 0, 0, 0)
    expect(Math.abs(foci[0].x - 0.3)).toBeLessThan(0.08)
    expect(Math.abs(foci[0].y - 0.6)).toBeLessThan(0.08)
    expect(packFoci(foci, new Array<number>(4).fill(0))).toEqual([foci[0].x, foci[0].y, 0.4, 0])
  })

  it('lean towards the pointer with inertia: heavier foci lag, all keep moving after it leaves', () => {
    const light = [createFocus(0.3, 0.5, 1, 0.5, 0.4)]
    const heavy = [createFocus(0.3, 0.5, 3, 0.5, 0.4)]
    const params = { stiffness: 4.5, damping: 1.6, orbit: 0, wander: 0 }
    for (let t = 0; t < 0.5; t += 1 / 60) {
      stepFoci(light, t, 1 / 60, 0.9, 0.5, 1, params)
      stepFoci(heavy, t, 1 / 60, 0.9, 0.5, 1, params)
    }
    expect(light[0].x).toBeGreaterThan(heavy[0].x)
    expect(light[0].x).toBeGreaterThan(0.3)
    const before = light[0].x
    stepFoci(light, 0.5, 1 / 60, 0, 0, 0, params)
    expect(light[0].x).toBeGreaterThan(before)
    for (let t = 0.5; t < 30; t += 1 / 60) stepFoci(light, t, 1 / 60, 0, 0, 0, params)
    expect(light[0].x).toBeCloseTo(0.3, 2)
  })
})

describe('particles', () => {
  const box = { x: 10, y: 20, w: 800, h: 180 }

  it('have deterministic constants in range', () => {
    const a = particleConstants(4, 12, 3)
    const b = particleConstants(4, 12, 3)
    expect(a).toEqual(b)
    expect(a).not.toEqual(particleConstants(4, 12, 4))
    for (let i = 0; i < 12; i++) {
      expect(a[i * 4]).toBeGreaterThanOrEqual(0)
      expect(a[i * 4]).toBeLessThan(1)
      expect(Math.abs(a[i * 4 + 1])).toBeLessThan(20)
      expect(a[i * 4 + 2]).toBeGreaterThan(0.1)
      expect(a[i * 4 + 2]).toBeLessThanOrEqual(1)
    }
    // Unused texels stay zero.
    expect(a.slice(48)).toEqual(new Float32Array(16))
    const r = lcg(1)
    expect(r()).not.toBe(r())
  })

  it('draw the waveform inside its box, breathing with time', () => {
    for (let x = 0; x <= 1; x += 0.01) {
      expect(Math.abs(waveform(x, 0))).toBeLessThanOrEqual(1)
      const h = homeOf(x, 0, 2, box)
      expect(h.y).toBeGreaterThan(box.y)
      expect(h.y).toBeLessThan(box.y + box.h)
    }
    expect(waveform(0, 0)).toBe(0)
    expect(waveform(0.1, 0)).not.toBe(waveform(0.1, 1))
  })

  it('mirror the GLSL simulation: same partials and envelope', () => {
    for (const piece of ['exp(-2.2 * x)', 'exp(-40.0 * x)', '0.86 + 0.14 * sin(t * 0.9)', 'x * 80.0 + t * 1.1', 'x * 211.0 - t * 2.3', 'x * 23.0 + t * 0.6']) {
      expect(SIM).toContain(piece)
    }
  })

  it('spring back home and settle (semi-implicit Euler, damped)', () => {
    const p = { x: 100, y: 100, vx: 0, vy: 0 }
    const home = { x: 140, y: 90 }
    const none = { x: -1e4, y: -1e4, vx: 0, vy: 0, weight: 0 }
    let maxX = 0
    for (let i = 0; i < 600; i++) {
      stepParticle(p, home, none, 1 / 60)
      maxX = Math.max(maxX, p.x)
      expect(Number.isFinite(p.x) && Number.isFinite(p.y)).toBe(true)
    }
    // It overshoots (inertia) then comes to rest at home.
    expect(maxX).toBeGreaterThan(home.x)
    expect(p.x).toBeCloseTo(home.x, 1)
    expect(p.y).toBeCloseTo(home.y, 1)
    expect(Math.hypot(p.vx, p.vy)).toBeLessThan(0.01)
  })

  it('are pushed away from the pointer, swirled, and dragged by its wake, within its radius only', () => {
    const home = { x: 100, y: 100 }
    const near = { x: 100, y: 100, vx: 0, vy: 0 }
    stepParticle(near, home, { x: 80, y: 100, vx: 0, vy: 0, weight: 1 }, 1 / 60)
    expect(near.vx).toBeGreaterThan(0)
    expect(near.vy).not.toBe(0)
    const far = { x: 100, y: 100, vx: 0, vy: 0 }
    stepParticle(far, home, { x: 80 - 200, y: 100, vx: 0, vy: 0, weight: 1 }, 1 / 60)
    expect(far.vx).toBe(0)
    const wake = { x: 100, y: 100, vx: 0, vy: 0 }
    stepParticle(wake, home, { x: 100, y: 90, vx: 0, vy: 500, weight: 1 }, 1 / 60)
    expect(wake.vy).toBeGreaterThan(0)
    const off = { x: 100, y: 100, vx: 0, vy: 0 }
    stepParticle(off, home, { x: 80, y: 100, vx: 0, vy: 0, weight: 0 }, 1 / 60)
    expect(off.vx).toBe(0)
  })
})

describe('pointer helpers', () => {
  it('keeps the idle pointer inside its box', () => {
    const box = { x: 100, y: 50, w: 400, h: 200 }
    for (let t = 0; t < 200; t += 0.37) {
      const p = idlePoint(t, box)
      expect(p.x).toBeGreaterThanOrEqual(box.x)
      expect(p.x).toBeLessThanOrEqual(box.x + box.w)
      expect(p.y).toBeGreaterThanOrEqual(box.y)
      expect(p.y).toBeLessThanOrEqual(box.y + box.h)
    }
  })

  it('approaches a target exponentially, exactly at dt 0', () => {
    expect(approach(0, 1, 5, 0)).toBe(0)
    const half = approach(0, 1, 5, 0.2)
    expect(half).toBeGreaterThan(0.5)
    expect(half).toBeLessThan(1)
    expect(approach(0, 1, 5, 10)).toBeCloseTo(1)
  })
})

describe('seek harness', () => {
  it('parses a bounded time and an optional pointer', () => {
    expect(seekRequest('')).toBeUndefined()
    expect(seekRequest('?fk-anim-t=abc')).toBeUndefined()
    expect(seekRequest('?fk-anim-t=-1')).toBeUndefined()
    expect(seekRequest('?fk-anim-t=2.5')).toEqual({ t: 2.5, x: -1000, y: -1000 })
    expect(seekRequest('?fk-anim-t=999&fk-anim-p=300,420')).toEqual({ t: 120, x: 300, y: 420 })
    expect(seekRequest('?fk-anim-t=1&fk-anim-p=x,y')).toEqual({ t: 1, x: -1000, y: -1000 })
  })
})

describe('shaders', () => {
  /** Every `smoothstep(e0, e1, …)` whose edges can be compared: plain numbers, or one expression plus constants. */
  function edgePairs(source: string): { call: string; e0: number; e1: number }[] {
    const pairs: { call: string; e0: number; e1: number }[] = []
    const term = (s: string) => {
      const m = /^(.*?)\s*([+-])\s*(\d*\.?\d+)$/.exec(s.trim())
      if (m && m[1] !== '') return { base: m[1].trim(), k: (m[2] === '-' ? -1 : 1) * Number(m[3]) }
      return Number.isFinite(Number(s)) ? { base: '', k: Number(s) } : { base: s.trim(), k: 0 }
    }
    for (const m of source.matchAll(/smoothstep\(([^,()]+),([^,()]+),/g)) {
      const a = term(m[1])
      const b = term(m[2])
      if (a.base === b.base) pairs.push({ call: m[0], e0: a.k, e1: b.k })
    }
    return pairs
  }

  it('never call smoothstep with edge0 >= edge1 (undefined in GLSL)', () => {
    const files = readdirSync(new URL('./shaders/', dir)).filter((f) => f.endsWith('.ts'))
    let checked = 0
    for (const file of files) {
      for (const { call, e0, e1 } of edgePairs(readFileSync(new URL(`./shaders/${file}`, dir), 'utf8'))) {
        checked++
        expect(e0 < e1, `${file}: ${call}`).toBe(true)
      }
    }
    expect(checked).toBeGreaterThan(10)
  })
})
