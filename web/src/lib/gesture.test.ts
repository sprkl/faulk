// @vitest-environment jsdom
import { describe, expect, it, vi } from 'vitest'
import { createGesture } from './gesture'

const types = (run: ReturnType<typeof vi.fn>) => run.mock.calls.map(([command]) => command.type)

describe('a fader gesture', () => {
  it('opens one history group and closes it once, whatever the number of begins and ends', () => {
    const run = vi.fn()
    const gesture = createGesture(run)
    gesture.end()
    expect(run).not.toHaveBeenCalled()
    gesture.begin()
    gesture.begin()
    expect(gesture.open).toBe(true)
    gesture.end()
    gesture.end()
    expect(gesture.open).toBe(false)
    expect(types(run)).toEqual(['history.begin_group', 'history.end_group'])
  })

  it('ends when the pointer is released anywhere, or cancelled', () => {
    for (const release of ['pointerup', 'pointercancel']) {
      const run = vi.fn()
      const gesture = createGesture(run)
      gesture.begin()
      window.dispatchEvent(new Event(release))
      expect(types(run), release).toEqual(['history.begin_group', 'history.end_group'])
      // The listeners are gone: a later release changes nothing.
      window.dispatchEvent(new Event(release))
      expect(run).toHaveBeenCalledTimes(2)
    }
  })

  it('can open again after it ended', () => {
    const run = vi.fn()
    const gesture = createGesture(run)
    gesture.begin()
    gesture.end()
    gesture.begin()
    gesture.end()
    expect(types(run)).toEqual(['history.begin_group', 'history.end_group', 'history.begin_group', 'history.end_group'])
  })
})
