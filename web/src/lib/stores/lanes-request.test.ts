import { describe, expect, it } from 'vitest'
import { UiStore } from './ui.svelte'

// #412: the one-shot request to unfold an owner's automation lanes in the arrangement.
describe('the lanes request', () => {
  it('navigates to the arrangement and is consumed once', () => {
    const ui = new UiStore('play', 'devices')
    ui.showLanes('t1')
    expect(ui.view).toBe('arrangement')
    expect(ui.takeLanesRequest()).toBe('t1')
    expect(ui.takeLanesRequest()).toBeNull()
  })

  it('is replaced by a second request before it is consumed', () => {
    const ui = new UiStore('play', 'devices')
    ui.showLanes('t1')
    ui.showLanes('t2')
    expect(ui.takeLanesRequest()).toBe('t2')
    expect(ui.takeLanesRequest()).toBeNull()
  })

  it('asks for the master with showMasterLanes', () => {
    const ui = new UiStore('play', 'devices')
    ui.showMasterLanes()
    expect(ui.takeLanesRequest()).toBe('master')
  })
})
