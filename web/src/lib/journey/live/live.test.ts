import { readdirSync, readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'
import { journeyBootScript } from '../boot'
import { clock as pageClock } from '../format'
import { bandLevels, bandRange, follow } from './bands'
import { clock, fill, lengthOf, SEEK_PAGE, SEEK_STEP, seekByKey } from './time'

const dir = new URL('./', import.meta.url)

describe('live module', () => {
  it('imports nothing outside its folder (the chunk hash covers its sources only)', () => {
    const files = readdirSync(dir).filter((f) => f.endsWith('.ts') && !f.endsWith('.test.ts'))
    expect(files.length).toBeGreaterThanOrEqual(4)
    for (const file of files) {
      const source = readFileSync(new URL(file, dir), 'utf8')
      const specs = [...source.matchAll(/(?:from|import)\s*\(?\s*'([^']+)'/g)].map((m) => m[1])
      for (const spec of specs) expect(spec.startsWith('./'), `${file}: ${spec}`).toBe(true)
    }
  })

  it('boots from an inline module script that cannot close its tag', () => {
    expect(journeyBootScript('/journey/live.1234.js')).toBe('import("/journey/live.1234.js").then(function(m){m.mount()}).catch(function(){})')
    expect(journeyBootScript('</script>')).not.toContain('</script>')
  })
})

describe('player time', () => {
  it('writes m:ss like the page does', () => {
    for (const s of [0, 0.9, 59.99, 60, 128.83, 3599]) expect(clock(s)).toBe(pageClock(s))
    expect(clock(128.83)).toBe('2:08')
    expect(clock(NaN)).toBe('0:00')
    expect(clock(-3)).toBe('0:00')
    expect(clock(Infinity)).toBe('0:00')
  })

  it('fills the spoken value', () => {
    expect(fill('{elapsed} of {total}', { elapsed: '0:42', total: '2:09' })).toBe('0:42 of 2:09')
    expect(fill('{elapsed} {x}', { elapsed: '1:00' })).toBe('1:00 {x}')
  })

  it('moves with the keys, clamped to the track', () => {
    expect(seekByKey('ArrowRight', 10, 100)).toBe(10 + SEEK_STEP)
    expect(seekByKey('ArrowUp', 10, 100)).toBe(10 + SEEK_STEP)
    expect(seekByKey('ArrowLeft', 10, 100)).toBe(10 - SEEK_STEP)
    expect(seekByKey('ArrowDown', 2, 100)).toBe(0)
    expect(seekByKey('PageUp', 90, 100)).toBe(100)
    expect(seekByKey('PageDown', 50, 100)).toBe(50 - SEEK_PAGE)
    expect(seekByKey('Home', 50, 100)).toBe(0)
    expect(seekByKey('End', 50, 100)).toBe(100)
    expect(seekByKey('Tab', 50, 100)).toBeUndefined()
    expect(seekByKey('ArrowRight', NaN, NaN)).toBe(0)
  })

  it('takes the media’s length once known, the stated one before', () => {
    expect(lengthOf(NaN, 128.83)).toBe(128.83)
    expect(lengthOf(Infinity, 128.83)).toBe(128.83)
    expect(lengthOf(0, 128.83)).toBe(128.83)
    expect(lengthOf(129.1, 128.83)).toBe(129.1)
  })
})

describe('visualiser bands', () => {
  it('splits the spectrum into ordered, non-empty log bands inside it', () => {
    for (const count of [1, 3, 5, 8]) {
      let prevFrom = 0
      for (let i = 0; i < count; i++) {
        const [from, to] = bandRange(i, count, 1024)
        expect(to).toBeGreaterThan(from)
        expect(from).toBeGreaterThanOrEqual(1)
        expect(to).toBeLessThanOrEqual(1024)
        expect(from).toBeGreaterThanOrEqual(prevFrom)
        prevFrom = from
      }
    }
    // Log spacing: the low band is narrower than the high one.
    const low = bandRange(0, 5, 1024)
    const high = bandRange(4, 5, 1024)
    expect(low[1] - low[0]).toBeLessThan(high[1] - high[0])
  })

  it('reads levels 0..1, a full spectrum as 1, silence as 0', () => {
    expect([...bandLevels(new Uint8Array(1024).fill(255), new Float32Array(5))]).toEqual([1, 1, 1, 1, 1])
    expect([...bandLevels(new Uint8Array(1024), new Float32Array(5))]).toEqual([0, 0, 0, 0, 0])
    const lowOnly = new Uint8Array(1024)
    lowOnly.fill(255, 0, 4)
    const levels = bandLevels(lowOnly, new Float32Array(5))
    expect(levels[0]).toBeGreaterThan(0.5)
    expect(levels[4]).toBe(0)
  })

  it('follows a target fast up and slowly down, and never leaves 0..1', () => {
    const up = follow(0, 1, 0.05)
    const down = follow(1, 0, 0.05)
    expect(up).toBeGreaterThan(1 - down)
    expect(follow(0.5, 0.5, 0.016)).toBeCloseTo(0.5)
    expect(follow(0, 5, 1)).toBeLessThanOrEqual(1)
    expect(follow(NaN, NaN, NaN)).toBe(0)
    expect(follow(0.3, 1, 0)).toBe(0.3)
  })
})
