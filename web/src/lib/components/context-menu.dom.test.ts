// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { MenuItem } from '../context-menu'
import { UiStore } from '../stores/ui.svelte'
import ContextMenu from './ContextMenu.svelte'

/** The shared context menu (#524): placement, keys, focus, and the store call that opens it. */

let host: HTMLDivElement
let app: ReturnType<typeof mount> | null
let closed: number
let ran: string[]

const items = (): MenuItem[] => [
  { id: 'a', label: 'First', shortcut: 'Ctrl+D', run: () => ran.push('a') },
  { id: 'b', label: 'Second', disabled: true, separator: true, run: () => ran.push('b') },
  { id: 'c', label: 'Third', detail: '64', run: () => ran.push('c') },
]

async function open(list: MenuItem[] = items(), onclose: () => void = () => closed++) {
  app = mount(ContextMenu, { target: host, props: { x: 40, y: 50, items: list, label: 'Actions', onclose } })
  flushSync()
  await Promise.resolve()
  await Promise.resolve()
  flushSync()
}
const menu = () => host.querySelector('[role="menu"]') as HTMLElement
const entries = () => [...host.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')]
const key = (name: string) => menu().dispatchEvent(new KeyboardEvent('keydown', { key: name, bubbles: true, cancelable: true }))

beforeEach(() => {
  closed = 0
  ran = []
  host = document.createElement('div')
  document.body.append(host)
})
afterEach(async () => {
  if (app) await unmount(app)
  app = null
  host.remove()
  vi.restoreAllMocks()
})

describe('the context menu', () => {
  it('is a labelled menu of buttons, with the shortcut or the detail of each entry at the right', async () => {
    await open()
    expect(menu().getAttribute('aria-label')).toBe('Actions')
    expect(entries().map((e) => e.textContent?.replace(/\s+/g, ' ').trim())).toEqual(['First Ctrl+D', 'Second', 'Third 64'])
    expect(host.querySelector('[role="separator"]')).not.toBeNull()
    expect(entries()[1]!.disabled).toBe(true)
  })

  it('puts the focus on the first enabled entry', async () => {
    await open()
    expect(document.activeElement).toBe(entries()[0])
  })

  it('moves with the arrows, skipping a disabled entry and wrapping, and with Home and End', async () => {
    await open()
    key('ArrowDown')
    expect(document.activeElement).toBe(entries()[2])
    key('ArrowDown')
    expect(document.activeElement).toBe(entries()[0])
    key('ArrowUp')
    expect(document.activeElement).toBe(entries()[2])
    key('Home')
    expect(document.activeElement).toBe(entries()[0])
    key('End')
    expect(document.activeElement).toBe(entries()[2])
  })

  it('runs the entry that is picked and closes, once; a disabled entry does nothing', async () => {
    await open()
    entries()[2]!.click()
    expect(ran).toEqual(['c'])
    expect(closed).toBe(1)
    entries()[1]!.click()
    expect(ran).toEqual(['c'])
  })

  it('closes with Escape without running anything, and with Tab', async () => {
    await open()
    key('Escape')
    expect(closed).toBe(1)
    key('Tab')
    expect(closed).toBe(2)
    expect(ran).toEqual([])
  })

  it('closes on a press outside, not on a press inside', async () => {
    await open()
    menu().dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }))
    expect(closed).toBe(0)
    document.body.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }))
    expect(closed).toBe(1)
  })

  it('cancels a right click on itself', async () => {
    await open()
    const event = new MouseEvent('contextmenu', { bubbles: true, cancelable: true })
    entries()[0]!.dispatchEvent(event)
    expect(event.defaultPrevented).toBe(true)
  })

  it('hands the focus back to the element that had it', async () => {
    const opener = document.createElement('button')
    document.body.append(opener)
    opener.focus()
    // The shell takes the menu out of the page when it closes.
    await open(items(), () => {
      void unmount(app!)
      app = null
    })
    expect(document.activeElement).toBe(entries()[0])
    key('Escape')
    await Promise.resolve()
    await Promise.resolve()
    expect(document.activeElement).toBe(opener)
    opener.remove()
  })
})

describe('opening it from the store', () => {
  const event = (target: EventTarget | null) => ({ clientX: 10, clientY: 20, target, preventDefault: vi.fn(), stopPropagation: vi.fn() })

  it('takes the event and opens the entries at the pointer', () => {
    const ui = new UiStore('arrangement', 'devices')
    const e = event(document.createElement('div'))
    ui.openMenu(e, items(), 'Actions')
    expect(e.preventDefault).toHaveBeenCalled()
    expect(e.stopPropagation).toHaveBeenCalled()
    expect(ui.contextMenu).toMatchObject({ x: 10, y: 20, label: 'Actions' })
    expect(ui.contextMenu?.items).toHaveLength(3)
  })

  it('opens nothing without entries, still cancels the browser menu, and closes a menu that was open', () => {
    const ui = new UiStore('arrangement', 'devices')
    ui.openMenu(event(document.createElement('div')), items())
    const e = event(document.createElement('div'))
    ui.openMenu(e, [])
    expect(e.preventDefault).toHaveBeenCalled()
    expect(ui.contextMenu).toBeNull()
  })

  it('leaves a text field its own menu', () => {
    const ui = new UiStore('arrangement', 'devices')
    const field = document.createElement('input')
    field.type = 'text'
    const e = event(field)
    ui.openMenu(e, items())
    expect(e.preventDefault).not.toHaveBeenCalled()
    expect(ui.contextMenu).toBeNull()
  })

  it('replaces the menu with a second list at a point', () => {
    const ui = new UiStore('arrangement', 'devices')
    ui.openMenu(event(document.createElement('div')), items())
    ui.openMenuAt(5, 6, [{ id: 'x', label: 'X', run: () => {} }], 'Colours')
    expect(ui.contextMenu).toMatchObject({ x: 5, y: 6, label: 'Colours' })
    ui.closeMenu()
    expect(ui.contextMenu).toBeNull()
  })
})
