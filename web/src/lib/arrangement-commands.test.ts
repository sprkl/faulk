import { describe, expect, it } from 'vitest'
import { barStart, moveToTrack, placeFromSlot, placeScene, repeatPlacement } from './arrangement-commands'

describe('sending the grid to the arrangement', () => {
  it('puts a clip on the bar the play position is in', () => {
    expect(barStart(0, 3840)).toBe(0)
    expect(barStart(3839, 3840)).toBe(0)
    expect(barStart(3840, 3840)).toBe(3840)
    expect(barStart(8180, 3840)).toBe(7680)
    expect(barStart(-5, 3840)).toBe(0)
    expect(barStart(100, 0)).toBe(0)
  })

  it('builds the two commands', () => {
    expect(placeFromSlot('c', 3840, false)).toEqual({ type: 'arrangement.place_from_slot', clip: 'c', start: 3840, linked: false })
    expect(placeScene('s', 0, true)).toEqual({ type: 'arrangement.place_scene', scene: 's', start: 0, linked: true })
  })
})

describe('repeating a placement', () => {
  it('names the placement and the tick it is repeated up to', () => {
    expect(repeatPlacement('p', 245760)).toEqual({ type: 'arrangement.repeat', placement: 'p', until: 245760 })
  })
})

describe('moving to another track', () => {
  it('sends the pairs, and the shift in time only when there is one', () => {
    const moves = [{ placement: 'p', track: 't' }]
    expect(moveToTrack(moves)).toEqual({ type: 'arrangement.move_to_track', moves })
    expect(moveToTrack(moves, 0)).toEqual({ type: 'arrangement.move_to_track', moves })
    expect(moveToTrack(moves, 960)).toEqual({ type: 'arrangement.move_to_track', moves, by: 960 })
  })
})
