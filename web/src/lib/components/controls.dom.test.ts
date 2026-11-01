// @vitest-environment jsdom
import { flushSync, mount, unmount, type Component } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Command } from '../generated/protocol'
import { UiStore } from '../stores/ui.svelte'
import { storesContext, type Stores } from '../stores'
import { setMasterPan, setMasterVolume, setPan, setVolume } from '../track-commands'
import GainControl from './GainControl.svelte'
import PanControl from './PanControl.svelte'

// The real dispatcher pulls in the wasm modules of the engine and the project, which these controls never touch.
vi.mock('../stores/dispatcher', () => ({ Dispatcher: class {} }))

/**
 * The shared control through its owners: what a drag, a double click, the context menu and the
 * keyboard send to the dispatcher. Layout does not exist in jsdom, so the length of a fader is given.
 */
const LENGTH = 168
let host: HTMLDivElement
let sent: Command[]
let app: ReturnType<typeof mount> | null

function setup(component: Component<never>, props: Record<string, unknown>) {
  // Only the user interface store and the dispatcher's `run` are read by these controls.
  const stores = { ui: new UiStore('play', 'devices'), dispatcher: { run: (command: Command) => void sent.push(command) } } as unknown as Stores
  app = mount(component as never, { target: host, props, context: storesContext(stores) })
  flushSync()
  const slider = host.querySelector('[role="slider"]') as HTMLElement
  Object.defineProperty(slider, 'clientHeight', { configurable: true, value: LENGTH })
  Object.defineProperty(slider, 'clientWidth', { configurable: true, value: LENGTH })
  slider.setPointerCapture = () => {}
  return slider
}

const pointer = (type: string, init: PointerEventInit & { clientX?: number; clientY?: number }) =>
  new PointerEvent(type, { bubbles: true, cancelable: true, pointerId: 1, button: 0, ...init })

function drag(slider: HTMLElement, to: { x?: number; y?: number }, modifiers: PointerEventInit = {}) {
  slider.dispatchEvent(pointer('pointerdown', { clientX: 100, clientY: 100 }))
  // The first move only passes the slop; the next ones change the value.
  slider.dispatchEvent(pointer('pointermove', { clientX: 104, clientY: 100, ...modifiers }))
  slider.dispatchEvent(pointer('pointermove', { clientX: 100 + (to.x ?? 0), clientY: 100 - (to.y ?? 0), ...modifiers }))
  slider.dispatchEvent(pointer('pointerup', { clientX: 100 + (to.x ?? 0), clientY: 100 - (to.y ?? 0) }))
  flushSync()
}

const types = () => sent.map((c) => c.type)

beforeEach(() => {
  sent = []
  host = document.createElement('div')
  document.body.append(host)
})
afterEach(async () => {
  if (app) await unmount(app)
  app = null
  host.remove()
  vi.restoreAllMocks()
})

describe('a volume control', () => {
  const gain = (value: number | null) => ({ gainDb: value, label: 'Volume', set: (g: number | null) => setVolume('t1', g), orientation: 'horizontal' })

  it('is a slider with a label and a value announced in dB', () => {
    const slider = setup(GainControl, gain(-6))
    expect(slider.getAttribute('aria-label')).toBe('Volume')
    expect(slider.getAttribute('aria-valuetext')).toBe('−6.0 dB')
    expect(slider.getAttribute('aria-valuemin')).toBe('0')
    expect(slider.getAttribute('aria-valuemax')).toBe('1')
    expect(slider.tabIndex).toBe(0)
  })

  it('puts 0 dB back on a double click as one command, and nothing when it is there already', () => {
    const slider = setup(GainControl, gain(-6))
    slider.dispatchEvent(new MouseEvent('dblclick', { bubbles: true }))
    expect(sent).toEqual([setVolume('t1', 0)])
    void unmount(app!)
    sent = []
    const at = setup(GainControl, gain(0))
    at.dispatchEvent(new MouseEvent('dblclick', { bubbles: true }))
    expect(sent).toEqual([])
  })

  it('does not reset on a double click whose second press was a drag', () => {
    const slider = setup(GainControl, gain(-30))
    drag(slider, { x: 40 })
    sent = []
    slider.dispatchEvent(new MouseEvent('dblclick', { bubbles: true }))
    expect(sent).toEqual([])
  })

  it('does not move on a press, a click or a double click (no intermediate step)', () => {
    const slider = setup(GainControl, gain(-6))
    for (let i = 0; i < 2; i++) {
      slider.dispatchEvent(pointer('pointerdown', { clientX: 50, clientY: 50 }))
      slider.dispatchEvent(pointer('pointermove', { clientX: 51, clientY: 50 }))
      slider.dispatchEvent(pointer('pointerup', { clientX: 51, clientY: 50 }))
    }
    slider.dispatchEvent(new MouseEvent('dblclick', { bubbles: true }))
    expect(types()).toEqual(['mixer.set_volume'])
    expect(sent[0]).toEqual(setVolume('t1', 0))
  })

  it('makes one history group of a drag, with the commands inside', () => {
    const slider = setup(GainControl, gain(-30))
    drag(slider, { x: 30 })
    expect(types()[0]).toBe('history.begin_group')
    expect(types().at(-1)).toBe('history.end_group')
    expect(types().filter((t) => t === 'history.begin_group')).toHaveLength(1)
    expect(
      types()
        .slice(1, -1)
        .every((t) => t === 'mixer.set_volume'),
    ).toBe(true)
    expect(types().length).toBeGreaterThan(2)
  })

  it('opens the menu on a right click and resets from it as one command, outside any group', () => {
    const slider = setup(GainControl, gain(-6))
    slider.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 10, clientY: 10 }))
    flushSync()
    const item = document.querySelector('[data-testid="value-menu-default"]') as HTMLButtonElement
    expect(item).not.toBeNull()
    expect(item.textContent).toContain('Default value')
    expect(item.textContent).toContain('0.0 dB')
    item.click()
    flushSync()
    expect(sent).toEqual([setVolume('t1', 0)])
    expect(document.querySelector('[data-testid="value-menu"]')).toBeNull()
  })

  it('closes the menu with Escape without sending anything', () => {
    const slider = setup(GainControl, gain(-6))
    slider.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 10, clientY: 10 }))
    flushSync()
    document.querySelector('[data-testid="value-menu"]')!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
    flushSync()
    expect(document.querySelector('[data-testid="value-menu"]')).toBeNull()
    expect(sent).toEqual([])
  })

  it('resets with Delete and Backspace, steps with the arrows, and ignores a key pressed during a drag', () => {
    const slider = setup(GainControl, gain(-6))
    slider.dispatchEvent(new KeyboardEvent('keydown', { key: 'Delete', bubbles: true, cancelable: true }))
    slider.dispatchEvent(new KeyboardEvent('keydown', { key: 'Backspace', bubbles: true, cancelable: true }))
    expect(sent).toEqual([setVolume('t1', 0), setVolume('t1', 0)])
    sent = []
    const up = new KeyboardEvent('keydown', { key: 'ArrowUp', bubbles: true, cancelable: true })
    slider.dispatchEvent(up)
    expect(up.defaultPrevented).toBe(true)
    expect(types()).toEqual(['mixer.set_volume'])
    // A letter is left alone: it plays a note.
    const letter = new KeyboardEvent('keydown', { key: 'a', bubbles: true, cancelable: true })
    slider.dispatchEvent(letter)
    expect(letter.defaultPrevented).toBe(false)
    expect(types()).toEqual(['mixer.set_volume'])
    // During a drag, a reset would land inside the history group: refused.
    sent = []
    slider.dispatchEvent(pointer('pointerdown', { clientX: 100, clientY: 100 }))
    slider.dispatchEvent(pointer('pointermove', { clientX: 130, clientY: 100 }))
    const before = sent.length
    slider.dispatchEvent(new KeyboardEvent('keydown', { key: 'Delete', bubbles: true, cancelable: true }))
    expect(sent.length).toBe(before)
    slider.dispatchEvent(pointer('pointerup', {}))
  })

  it('stops on 0 dB when a drag goes through it (the fine mode is tested in continuous.test.ts and in the browser)', () => {
    // From -6 dB (position 0.64) to 0 dB (0.78) is 21 px of a 152 px travel.
    const slider = setup(GainControl, gain(-6))
    const to = (px: number, ctrl = false) => {
      sent = []
      slider.dispatchEvent(pointer('pointerdown', { clientX: 100, clientY: 100 }))
      slider.dispatchEvent(pointer('pointermove', { clientX: 104, clientY: 100, ctrlKey: ctrl }))
      slider.dispatchEvent(pointer('pointermove', { clientX: 104 + px, clientY: 100, ctrlKey: ctrl }))
      const volumes = sent.filter((c) => c.type === 'mixer.set_volume') as Extract<Command, { type: 'mixer.set_volume' }>[]
      slider.dispatchEvent(pointer('pointerup', {}))
      return volumes.at(-1)?.gain_db
    }
    // Sweeping the pointer: the value sits on 0 dB exactly for the whole plateau (10 px), and goes past it.
    const sweep = Array.from({ length: 80 }, (_, px) => to(px))
    expect(sweep.filter((g) => g === 0).length).toBeGreaterThanOrEqual(10)
    expect(sweep.at(-1)!).toBeGreaterThan(0)
  })
})

describe('the master volume', () => {
  it('resets through the master command', () => {
    const slider = setup(GainControl, { gainDb: 3, label: 'Master volume', set: setMasterVolume, orientation: 'vertical' })
    slider.dispatchEvent(new MouseEvent('dblclick', { bubbles: true }))
    expect(sent).toEqual([setMasterVolume(0)])
  })
})

describe('a pan control', () => {
  const pan = (value: number) => ({ track: 't1', pan: value, label: 'Pan' })

  it('announces the pan, and centres on a double click as one command (none when centred)', () => {
    const slider = setup(PanControl, pan(0.4))
    expect(slider.getAttribute('aria-valuetext')).toBe('R40')
    slider.dispatchEvent(new MouseEvent('dblclick', { bubbles: true }))
    expect(sent).toEqual([setPan('t1', 0)])
    void unmount(app!)
    sent = []
    const centred = setup(PanControl, pan(0))
    centred.dispatchEvent(new MouseEvent('dblclick', { bubbles: true }))
    expect(sent).toEqual([])
  })

  it('sends the master pan command when it has no track, and centres it on a double click', () => {
    const slider = setup(PanControl, { track: null, pan: -0.5, label: 'Master pan' })
    slider.dispatchEvent(new MouseEvent('dblclick', { bubbles: true }))
    expect(sent).toEqual([setMasterPan(0)])
  })

  it('shows the centre in the menu and resets from it', () => {
    const slider = setup(PanControl, pan(-0.5))
    slider.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 10, clientY: 10 }))
    flushSync()
    const item = document.querySelector('[data-testid="value-menu-default"]') as HTMLButtonElement
    expect(item.textContent).toContain('C')
    item.click()
    expect(sent).toEqual([setPan('t1', 0)])
  })
})
