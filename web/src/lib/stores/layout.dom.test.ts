import { beforeEach, describe, expect, it, vi } from 'vitest'
import { LAYOUT_INDEX, LAYOUT_PREFIX, MAX_PROJECTS, readLayout, writeLayout } from './layout'
import { UiStore } from './ui.svelte'
import type { MainViewId } from '../../views/registry'

const isView = (id: string): id is MainViewId => ['arrangement', 'grid', 'mixer'].includes(id)

describe("each project's layout (#473)", () => {
  beforeEach(() => {
    localStorage.clear()
    vi.useRealTimers()
  })

  it('reads back what was written, and drops the least recently used past the bound', () => {
    writeLayout('p0', { view: 'mixer', memory: { arrangement: { scrollY: 40 } } })
    expect(readLayout('p0')).toEqual({ view: 'mixer', memory: { arrangement: { scrollY: 40 } } })
    for (let i = 1; i <= MAX_PROJECTS; i++) writeLayout(`p${i}`, { view: 'grid' })
    expect(readLayout('p0')).toBeNull()
    expect(localStorage.getItem(LAYOUT_PREFIX + 'p0')).toBeNull()
    expect(readLayout('p1')).not.toBeNull()
    expect(JSON.parse(localStorage.getItem(LAYOUT_INDEX)!)).toHaveLength(MAX_PROJECTS)
  })

  it('reads a damaged record as none, and keeps only the parts that read', () => {
    localStorage.setItem(LAYOUT_PREFIX + 'bad', '{not json')
    expect(readLayout('bad')).toBeNull()
    localStorage.setItem(LAYOUT_PREFIX + 'odd', JSON.stringify({ view: 3, laneHeights: { a: 'x', b: 50 }, memory: [] }))
    // The lane heights an earlier build kept here are the project's now (#490): ignored.
    expect(readLayout('odd')).toEqual({})
  })

  it('brings a project back with its view and memory; another project starts from the defaults', () => {
    const ui = new UiStore('arrangement', 'devices')
    ui.useProject('song', isView)
    ui.show('grid')
    ui.remember('arrangement', { scrollY: 120 })
    ui.keepNow()

    const later = new UiStore('arrangement', 'devices')
    later.useProject('song', isView)
    expect(later.view).toBe('grid')
    expect(later.recall('arrangement')).toEqual({ scrollY: 120 })
    const rev = later.layoutRev
    later.useProject('other', isView)
    expect(later.recall('arrangement')).toBeUndefined()
    expect(later.layoutRev).toBe(rev + 1)
    // The same project again changes nothing.
    later.useProject('other', isView)
    expect(later.layoutRev).toBe(rev + 1)
  })

  it('ignores what a view of the previous layout hands over as it closes', () => {
    const ui = new UiStore('arrangement', 'devices')
    ui.useProject('a', isView)
    const opened = ui.layoutRev
    ui.useProject('b', isView)
    ui.remember('arrangement', { scrollY: 999 }, opened)
    expect(ui.recall('arrangement')).toBeUndefined()
    ui.remember('arrangement', { scrollY: 5 }, ui.layoutRev)
    expect(ui.recall('arrangement')).toEqual({ scrollY: 5 })
  })

  it('keeps a moment after the last change, and carries the layout to a copy', () => {
    vi.useFakeTimers()
    const ui = new UiStore('arrangement', 'devices')
    ui.useProject('draft', isView)
    ui.remember('grid', { top: 10, left: 0 })
    expect(readLayout('draft')?.memory).toBeUndefined()
    vi.advanceTimersByTime(500)
    expect(readLayout('draft')?.memory).toEqual({ grid: { top: 10, left: 0 } })
    ui.carryLayout('copy')
    expect(readLayout('copy')?.memory).toEqual({ grid: { top: 10, left: 0 } })
    // The copy's own layout is the one shown now: opening it changes nothing.
    const rev = ui.layoutRev
    ui.useProject('copy', isView)
    expect(ui.layoutRev).toBe(rev)
  })

  it('a view that no longer exists is not restored', () => {
    writeLayout('p', { view: 'gone' })
    const ui = new UiStore('arrangement', 'devices')
    ui.useProject('p', isView)
    expect(ui.view).toBe('arrangement')
  })
})
