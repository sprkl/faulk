// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { DeviceDescriptor, ParamDescriptor } from '../devices'
import { storesContext, type Stores } from '../stores'
import { UiStore } from '../stores/ui.svelte'
import ParamControl from './ParamControl.svelte'

// The real dispatcher pulls in the wasm modules of the engine and the project, which a knob never touches.
vi.mock('../stores/dispatcher', () => ({ Dispatcher: class {} }))

/** A device knob through its descriptor: the default value comes from it, and the magnet holds it exactly. */
const device = { kind: 'faulk.fake_fx', kind_id: 99, version: 1, name: 'Fake FX', role: 'effect', memory_bytes: 0, midi_output: null, params: [] } as unknown as DeviceDescriptor
const log = { id: 1, key: 'time', description: 'Time.', unit: 'seconds', min: 0.01, max: 2, default: 0.3, scale: 'log', step: 0, smoothing_seconds: 0, values: [] } as unknown as ParamDescriptor

let host: HTMLDivElement
let app: ReturnType<typeof mount> | null
let changes: number[]
let began: number
let ended: number

function setup(value: number) {
  const stores = { ui: new UiStore('play', 'devices'), dispatcher: {} } as unknown as Stores
  app = mount(ParamControl as never, {
    target: host,
    props: { device, param: log, value, onchange: (v: number) => changes.push(v), onbegin: () => began++, onend: () => ended++ },
    context: storesContext(stores),
  })
  flushSync()
  const slider = host.querySelector('[role="slider"]') as HTMLElement
  slider.setPointerCapture = () => {}
  return slider
}

const pointer = (type: string, init: PointerEventInit) => new PointerEvent(type, { bubbles: true, cancelable: true, pointerId: 1, button: 0, ...init })

beforeEach(() => {
  changes = []
  began = 0
  ended = 0
  host = document.createElement('div')
  document.body.append(host)
})
afterEach(() => {
  if (app) unmount(app)
  app = null
  host.remove()
})

describe('a device knob', () => {
  it('sits on the default value exactly when a drag goes through it (a log parameter included)', () => {
    const slider = setup(0.05)
    slider.dispatchEvent(pointer('pointerdown', { clientX: 100, clientY: 100 }))
    slider.dispatchEvent(pointer('pointermove', { clientX: 104, clientY: 100 }))
    // The knob's travel is 160 px: sweep to the right and collect what it sends.
    for (let px = 0; px < 120; px++) slider.dispatchEvent(pointer('pointermove', { clientX: 104 + px, clientY: 100 }))
    slider.dispatchEvent(pointer('pointerup', {}))
    expect(changes).toContain(0.3)
    // The magnet holds it for several moves, and the sweep goes past it.
    expect(changes.filter((v) => v === 0.3).length).toBeGreaterThanOrEqual(5)
    expect(Math.max(...changes)).toBeGreaterThan(0.3)
    expect(began).toBe(1)
    expect(ended).toBe(1)
  })

  it('resets to the descriptor default as one change, and sends nothing when it is there', () => {
    const slider = setup(0.9)
    slider.dispatchEvent(new MouseEvent('dblclick', { bubbles: true }))
    expect(changes).toEqual([0.3])
    unmount(app!)
    changes = []
    const atDefault = setup(0.3)
    atDefault.dispatchEvent(new MouseEvent('dblclick', { bubbles: true }))
    expect(changes).toEqual([])
    expect(began).toBe(0)
  })

  it('announces its value and shows the default in the menu', () => {
    const slider = setup(0.9)
    expect(slider.getAttribute('aria-valuetext')).toBe('900 ms')
    slider.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 10, clientY: 10 }))
    flushSync()
    const item = document.querySelector('[data-testid="value-menu-default"]') as HTMLButtonElement
    expect(item.textContent).toContain('300 ms')
    // The menu is not inside a label: its name is its own.
    expect(item.closest('label')).toBeNull()
    item.click()
    expect(changes).toEqual([0.3])
  })
})
