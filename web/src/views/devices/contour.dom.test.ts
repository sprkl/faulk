// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { descriptorOf } from '../../lib/devices'
import { storesContext, type Stores } from '../../lib/stores'
import { UiStore } from '../../lib/stores/ui.svelte'
import ContourCurve from './ContourCurve.svelte'

vi.mock('../../lib/stores/dispatcher', () => ({ Dispatcher: class {} }))

/**
 * The curve's gestures (#156): each one is `device.set_param` values through `onchange`, a drag and a
 * wheel gesture each one history group (`onbegin` … `onend`), a double click and Enter switch a band,
 * the arrows move a focused band.
 */
let host: HTMLDivElement
let app: ReturnType<typeof mount> | null = null
let values: Record<string, number>
let changes: [string, number][]
let groups: string[]

const descriptor = descriptorOf('faulk.contour')
const defaults = () => Object.fromEntries(descriptor.params.map((p) => [p.key, p.default]))

function mountCurve(spectrum: Parameters<typeof ContourCurve>[1]['spectrum'] = null) {
  const ui = new UiStore('arrangement', 'devices')
  ui.lang = 'en'
  const stores = { ui } as unknown as Stores
  app = mount(ContourCurve, {
    target: host,
    context: storesContext(stores),
    props: {
      descriptor,
      name: 'Contour',
      valueOf: (key: string) => values[key],
      spectrum,
      sampleRate: 48000,
      onchange: (key: string, value: number) => {
        changes.push([key, value])
        values[key] = value
      },
      onbegin: () => groups.push('begin'),
      onend: () => groups.push('end'),
    },
  })
  flushSync()
}

const handle = (band: string) => host.querySelector<SVGGElement>(`[data-testid="contour-handle"][data-band="${band}"]`)!
const svg = () => host.querySelector<SVGSVGElement>('svg')!

beforeEach(() => {
  values = defaults()
  changes = []
  groups = []
  // jsdom has no ResizeObserver (the curve follows its width with one).
  globalThis.ResizeObserver ??= class {
    observe() {}
    unobserve() {}
    disconnect() {}
  } as unknown as typeof ResizeObserver
  host = document.createElement('div')
  document.body.append(host)
  // jsdom lays nothing out: the curve is 480 px wide and 150 px high at the page's origin.
  vi.spyOn(Element.prototype, 'getBoundingClientRect').mockReturnValue({
    left: 0,
    top: 0,
    width: 480,
    height: 150,
    right: 480,
    bottom: 150,
    x: 0,
    y: 0,
    toJSON: () => ({}),
  })
})

afterEach(() => {
  if (app) void unmount(app)
  app = null
  host.remove()
  vi.restoreAllMocks()
  vi.useRealTimers()
})

const pointer = (type: string, x: number, y: number) => new PointerEvent(type, { bubbles: true, clientX: x, clientY: y, button: 0, pointerId: 1 })

describe('the Contour curve', () => {
  it('shows a handle per band, the curve, and the spectrum only when there is one', () => {
    mountCurve()
    expect(host.querySelectorAll('[data-testid="contour-handle"]')).toHaveLength(7)
    expect(host.querySelector('[data-testid="contour-response"]')?.getAttribute('d')).toMatch(/^M0\.0 /)
    expect(host.querySelector('[data-testid="contour-spectrum"]')).toBeNull()
    expect(handle('bell2').getAttribute('aria-label')).toBe('Bell 2, 1.00 kHz, 0.0 dB, Q 1.00')
    expect(handle('hp').getAttribute('aria-label')).toBe('Low cut, 30 Hz, Q 0.71, off')
    void unmount(app!)
    app = null
    mountCurve({ bands: new Float32Array(96).fill(-30), sampleRate: 48000, at: 0 })
    expect(host.querySelector('[data-testid="contour-spectrum"]')).not.toBeNull()
    expect(host.querySelector('[data-testid="contour-curve"]')?.getAttribute('data-spectrum')).toBe('on')
  })

  it('makes a drag one group of frequency and gain changes, by the movement from where the handle was grabbed', () => {
    if (!('PointerEvent' in globalThis)) globalThis.PointerEvent = MouseEvent as unknown as typeof PointerEvent
    mountCurve()
    // Grabbed off its centre (bell 2 sits at 1 kHz, 0 dB): nothing moves until the pointer does.
    handle('bell2').dispatchEvent(pointer('pointerdown', 250, 80))
    // Within the dead zone (2 px): nothing sent.
    svg().dispatchEvent(pointer('pointermove', 252, 80))
    expect(changes).toEqual([])
    // 37.5 px up is 37.5 / 67 of +24 dB (the curve keeps 8 px at each edge): +13.4 dB; the frequency stays.
    svg().dispatchEvent(pointer('pointermove', 250, 42.5))
    // A quarter of the width right is a quarter of the three decades: 1 kHz x 1000^0.25 = 5.62 kHz.
    svg().dispatchEvent(pointer('pointermove', 370, 42.5))
    svg().dispatchEvent(pointer('pointerup', 370, 42.5))
    expect(groups).toEqual(['begin', 'end'])
    const freqs = changes.filter(([k]) => k === 'bell2_freq').map(([, v]) => v)
    const gains = changes.filter(([k]) => k === 'bell2_gain').map(([, v]) => v)
    expect(freqs).toHaveLength(1)
    expect(freqs[0]).toBeCloseTo(1000 * Math.pow(1000, 0.25), 0)
    // The gain moved once (the second move kept the height: nothing sent for it).
    expect(gains).toHaveLength(1)
    expect(gains[0]).toBeCloseTo(13.4, 1)
    // A click without a move changes nothing, and still closes its group.
    changes = []
    groups = []
    handle('bell1').dispatchEvent(pointer('pointerdown', 100, 75))
    svg().dispatchEvent(pointer('pointerup', 101, 75))
    expect(changes).toEqual([])
    expect(groups).toEqual(['begin', 'end'])
    // A cut band moves its frequency only.
    changes = []
    handle('hp').dispatchEvent(pointer('pointerdown', 0, 75))
    svg().dispatchEvent(pointer('pointermove', 120, 10))
    svg().dispatchEvent(pointer('pointerup', 120, 10))
    expect(changes.map(([k]) => k)).toEqual(['hp_freq'])
  })

  it('makes a wheel gesture one group of Q changes, closed once the wheel rests', () => {
    vi.useFakeTimers()
    mountCurve()
    for (let i = 0; i < 3; i++) handle('bell1').dispatchEvent(new WheelEvent('wheel', { deltaY: -100, bubbles: true, cancelable: true }))
    // Every tick asks for the group (the card's gesture opens one only when none is open), none ends it.
    expect(groups).toEqual(['begin', 'begin', 'begin'])
    vi.advanceTimersByTime(500)
    expect(groups).toEqual(['begin', 'begin', 'begin', 'end'])
    const qs = changes.filter(([k]) => k === 'bell1_q').map(([, v]) => v)
    expect(qs).toHaveLength(3)
    expect(qs[2]).toBeCloseTo(Math.pow(2, 0.75), 2)
  })

  it('switches a band on a double click and on Enter, and moves a focused one with the arrows', () => {
    mountCurve()
    handle('lp').dispatchEvent(new MouseEvent('dblclick', { bubbles: true }))
    expect(changes).toEqual([['lp_on', 1]])
    handle('lp').dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }))
    expect(changes.at(-1)).toEqual(['lp_on', 0])
    changes = []
    const key = (k: string, shiftKey = false) => handle('bell3').dispatchEvent(new KeyboardEvent('keydown', { key: k, shiftKey, bubbles: true }))
    key('ArrowUp')
    key('ArrowRight')
    key('ArrowUp', true)
    expect(changes[0]).toEqual(['bell3_gain', 0.5])
    expect(changes[1][0]).toBe('bell3_freq')
    expect(changes[1][1]).toBeCloseTo(4000 * Math.pow(2, 1 / 6), 0)
    expect(changes[2]).toEqual(['bell3_q', 1.1])
    // A cut band's up and down set its Q.
    handle('hp').dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true }))
    expect(changes.at(-1)?.[0]).toBe('hp_q')
    // Held arrows are one group: opened by the first, closed when the key comes up (or on blur).
    expect(groups).toEqual(['begin'])
    handle('hp').dispatchEvent(new KeyboardEvent('keyup', { key: 'ArrowDown', bubbles: true }))
    expect(groups).toEqual(['begin', 'end'])
    key('ArrowLeft')
    handle('bell3').dispatchEvent(new FocusEvent('blur'))
    expect(groups).toEqual(['begin', 'end', 'begin', 'end'])
    // Enter switches a band in one step, no group.
    handle('lp').dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }))
    expect(groups).toHaveLength(4)
  })
})
