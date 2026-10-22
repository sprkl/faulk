import type { Command, Quantize } from './generated/protocol'

/** The commands of the transport bar: each control is one of them (or a short group of them). */
export const play = (): Command => ({ type: 'transport.play' })
export const stop = (): Command => ({ type: 'transport.stop' })
export const seek = (tick: number): Command => ({ type: 'transport.seek', tick })
export const setTempo = (bpm: number): Command => ({ type: 'transport.set_tempo', bpm })
export const setTimeSignature = (numerator: number, denominator: number): Command => ({
  type: 'transport.set_time_signature',
  numerator,
  denominator,
})
export const setLoopRegion = (start: number, end: number): Command => ({ type: 'transport.set_loop', loop_region: { start, end } })
export const toggleLoop = (): Command => ({ type: 'transport.toggle_loop' })
export const setMetronome = (enabled: boolean): Command => ({ type: 'transport.set_metronome', enabled })
export const setLaunchQuantize = (quantize: Quantize): Command => ({ type: 'transport.set_launch_quantize', quantize })
