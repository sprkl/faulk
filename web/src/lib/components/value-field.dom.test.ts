// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { DeviceDescriptor, ParamDescriptor } from '../devices'
import { storesContext, type Stores } from '../stores'
import { UiStore } from '../stores/ui.svelte'
import ParamControl from './ParamControl.svelte'

// The real dispatcher pulls in the wasm modules of the engine and the project, which a knob never touches.
vi.mock('../stores/dispatcher', () => ({ Dispatcher: class {} }))

// #365: typing a value into a control, through a device knob (the field is the one of every control: `Slider`).
const device = {
  kind: 'faulk.fake_fx',
  kind_id: 99,
  version: 1,
  name: 'Fake FX',
  role: 'effect',
  memory_bytes: 0,
  midi_output: null,
  params: [],
} as unknown as DeviceDescriptor
const time = {
  id: 1,
  key: 'time',
  description: 'Time.',
  unit: 'seconds',
  min: 0.01,
  max: 2,
  default: 0.3,
  scale: 'log',
  step: 0,
  smoothing_seconds: 0,
  values: [],
} as unknown as ParamDescriptor

let host: HTMLDivElement
let app: ReturnType<typeof mount> | null
let changes: number[]

function setup(value: number) {
  const stores = { ui: new UiStore('play', 'devices'), dispatcher: {} } as unknown as Stores
  app = mount(ParamControl as never, {
    target: host,
    props: { device, param: time, value, onchange: (v: number) => changes.push(v) },
    context: storesContext(stores),
  })
  flushSync()
  return host.querySelector('[role="slider"]') as HTMLElement
}

beforeEach(() => {
  changes = []
  host = document.createElement('div')
  document.body.append(host)
})
afterEach(async () => {
  if (app) await unmount(app)
  app = null
  host.remove()
})

const field = () => host.querySelector('[data-testid="value-input"]') as HTMLInputElement | null
const type = (input: HTMLInputElement, text: string) => {
  input.value = text
  input.dispatchEvent(new Event('input', { bubbles: true }))
  flushSync()
}
const key = (input: HTMLInputElement, name: string) => {
  input.dispatchEvent(new KeyboardEvent('keydown', { key: name, bubbles: true, cancelable: true }))
  flushSync()
}
const open = (slider: HTMLElement) => {
  slider.focus()
  slider.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }))
  flushSync()
  return field()!
}

describe('typing a value (#365)', () => {
  it('opens on Enter with the displayed value, and Enter applies one change', () => {
    const slider = setup(0.9)
    const input = open(slider)
    expect(input.value).toBe('900 ms')
    expect(document.activeElement).toBe(input)
    type(input, '250 ms')
    key(input, 'Enter')
    expect(changes).toEqual([0.25])
    expect(field()).toBeNull()
    expect(document.activeElement).toBe(slider)
  })

  it('opens from the value readout', () => {
    setup(0.9)
    ;(host.querySelector('[data-testid="value-readout"]') as HTMLButtonElement).click()
    flushSync()
    expect(field()?.value).toBe('900 ms')
  })

  it('Escape cancels; opening alone applies nothing', () => {
    const slider = setup(0.9)
    const input = open(slider)
    expect(changes).toEqual([])
    type(input, '100')
    key(input, 'Escape')
    expect(changes).toEqual([])
    expect(field()).toBeNull()
    expect(document.activeElement).toBe(slider)
  })

  it('an unreadable text changes nothing, says so and keeps the field open', () => {
    const input = open(setup(0.9))
    type(input, 'loud')
    key(input, 'Enter')
    expect(changes).toEqual([])
    expect(field()).not.toBeNull()
    expect(field()!.getAttribute('aria-invalid')).toBe('true')
    expect(host.querySelector('[data-testid="value-error"]')).not.toBeNull()
    // Typing again clears the mark; a readable text applies.
    type(input, '1.5 s')
    expect(field()!.getAttribute('aria-invalid')).toBe('false')
    key(input, 'Enter')
    expect(changes).toEqual([1.5])
  })

  it('losing the focus applies a readable text; an unreadable one stays open and flagged until a press elsewhere', () => {
    let input = open(setup(0.9))
    type(input, '500')
    input.blur()
    flushSync()
    expect(changes).toEqual([0.5])
    expect(field()).toBeNull()
    void unmount(app!)
    changes = []
    input = open(setup(0.9))
    type(input, 'x')
    input.blur()
    flushSync()
    expect(changes).toEqual([])
    expect(field()!.getAttribute('aria-invalid')).toBe('true')
    document.body.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }))
    flushSync()
    expect(field()).toBeNull()
  })

  it('out of range comes back to the bound; a field left as it opened sends nothing', () => {
    let input = open(setup(0.9))
    type(input, '99 s')
    key(input, 'Enter')
    expect(changes).toEqual([2])
    void unmount(app!)
    changes = []
    // 0.9004 s reads "900 ms": a field left as it opened must not overwrite the finer value.
    input = open(setup(0.9004))
    expect(input.value).toBe('900 ms')
    key(input, 'Enter')
    expect(changes).toEqual([])
    expect(field()).toBeNull()
    // Typing the value the control already has sends nothing either.
    void unmount(app!)
    input = open(setup(2))
    type(input, '2 s')
    key(input, 'Enter')
    expect(changes).toEqual([])
  })

  it('takes every key: none reaches an ancestor', () => {
    const outer: string[] = []
    // A page-wide shortcut listens on the window; a delegated handler that stops the event keeps it from there.
    const seen = (e: KeyboardEvent) => outer.push(e.key)
    window.addEventListener('keydown', seen)
    const slider = setup(0.9)
    open(slider)
    outer.length = 0
    for (const k of ['a', ' ', 'Backspace', 'ArrowLeft', 'Delete', 'Enter']) key(field()!, k)
    expect(outer).toEqual([])
    // The Enter that opens the field is not seen above the control either.
    void unmount(app!)
    outer.length = 0
    const again = setup(0.9)
    open(again)
    window.removeEventListener('keydown', seen)
    expect(outer).toEqual([])
  })

  it('Save and the history pass through the field (Ctrl or Cmd), as in the other text fields', () => {
    const seen: string[] = []
    const listener = (e: KeyboardEvent) => seen.push(`${e.ctrlKey ? 'Ctrl+' : ''}${e.key}`)
    window.addEventListener('keydown', listener)
    const input = open(setup(0.9))
    input.dispatchEvent(new KeyboardEvent('keydown', { key: 's', ctrlKey: true, bubbles: true, cancelable: true }))
    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'z', ctrlKey: true, bubbles: true, cancelable: true }))
    window.removeEventListener('keydown', listener)
    expect(seen).toEqual(['Ctrl+s', 'Ctrl+z'])
  })

  it('an unreadable text left behind by the focus is dropped by the next key pressed elsewhere', () => {
    const input = open(setup(0.9))
    type(input, 'x')
    input.blur()
    flushSync()
    expect(field()).not.toBeNull()
    document.body.dispatchEvent(new KeyboardEvent('keydown', { key: 'a', bubbles: true, cancelable: true }))
    flushSync()
    expect(field()).toBeNull()
    expect(changes).toEqual([])
  })

  it('Ctrl+Enter, a composition and a held Enter do not apply, cancel or reopen the field', () => {
    const slider = setup(0.9)
    const input = open(slider)
    type(input, '100 ms')
    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', ctrlKey: true, bubbles: true, cancelable: true }))
    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', isComposing: true, bubbles: true, cancelable: true }))
    flushSync()
    expect(field()).not.toBeNull()
    expect(changes).toEqual([])
    key(input, 'Escape')
    expect(field()).toBeNull()
    slider.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', repeat: true, bubbles: true, cancelable: true }))
    flushSync()
    expect(field()).toBeNull()
  })

  it('a double click still resets to the default', () => {
    setup(0.9).dispatchEvent(new MouseEvent('dblclick', { bubbles: true }))
    expect(changes).toEqual([0.3])
  })
})
