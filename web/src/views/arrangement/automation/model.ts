/**
 * The automation lanes of a track as the timeline edits them (AM-4), as pure functions: the parameters a track
 * can automate, the vertical axis of a lane (its range, scale and unit, in the units of the command), the sorted
 * points of a lane, and the value of a lane at a tick (the engine's rule of AM-2, for the live readout and for the
 * controls that follow the playback). No DOM.
 */

import type { AutomationLane, AutomationTarget, MixerParam, Shape, Track } from '../../../lib/generated/protocol'
import {
  positionOf as devicePosition,
  snap as snapDevice,
  valueAt as deviceValueAt,
  formatValue,
  isChoice,
  type DeviceDescriptor,
  type ParamDescriptor,
} from '../../../lib/devices'
import { chainOf, type ChainEntry } from '../../../lib/project/chain'
import { exactDbAt, gainAt, positionOf as faderPosition } from '../../../lib/project/fader'
import { MAX_GAIN_DB, MIN_GAIN_DB, formatGain, formatPan } from '../../../lib/project/tracks'
import { parseGainEntry, parsePanEntry, parseParamEntry, parseQuantity } from '../../../lib/value-entry'

/** A point of a lane, with its tick: what the editor works on. */
export interface LanePoint {
  tick: number
  value: number
  shape: Shape
}

const sortedCache = new WeakMap<AutomationLane, readonly LanePoint[]>()

/** The points of a lane by tick, earliest first. The project replaces a lane's object when it changes, so the cache is by object. */
export function pointsOf(lane: AutomationLane): readonly LanePoint[] {
  let points = sortedCache.get(lane)
  if (!points) {
    const list: LanePoint[] = []
    for (const key of Object.keys(lane.points)) {
      const point = lane.points[key]
      if (point) list.push({ tick: Number(key), value: point.value, shape: point.shape })
    }
    list.sort((a, b) => a.tick - b.tick)
    points = list
    sortedCache.set(lane, points)
  }
  return points
}

/** The index of the first point at or after `tick` (the length when there is none). */
export function lowerBound(points: readonly LanePoint[], tick: number): number {
  let lo = 0
  let hi = points.length
  while (lo < hi) {
    const mid = (lo + hi) >>> 1
    if (points[mid]!.tick < tick) lo = mid + 1
    else hi = mid
  }
  return lo
}

/** The point on `tick`, or null. */
export function pointAtTick(points: readonly LanePoint[], tick: number): LanePoint | null {
  const i = lowerBound(points, tick)
  return points[i] && points[i]!.tick === tick ? points[i]! : null
}

/** A key for the parameter a lane automates: one lane per key and track. */
export const targetKey = (target: AutomationTarget): string =>
  'device' in target ? `d:${target.device.device}:${target.device.param}` : `m:${target.mixer.param}`

/** How the page names what it shows (the catalogue of the user's language). */
export interface Labels {
  mixer: string
  mixerParam: (param: MixerParam) => string
  /** A parameter's name, as the device panel shows it. */
  param: (descriptor: DeviceDescriptor, param: ParamDescriptor) => string
  /** The name of one value of a choice parameter. */
  choice: (descriptor: DeviceDescriptor, key: string) => string
}

/** The vertical axis of a lane: its range, the position of a value along it and the text of a value, all in the unit of the command. */
export interface Axis {
  /** Value to position, 0 at the bottom of the lane and 1 at the top. */
  pos: (value: number) => number
  /** Position to the value it stands for, kept to the range and rounded to the step as the command does. */
  at: (position: number) => number
  /** A value kept to the range and rounded to the step. */
  clamp: (value: number) => number
  min: number
  max: number
  /** The position of the value the parameter has when nothing is set. */
  home: number
  /** The text of a value, with its unit. */
  format: (value: number) => string
  /**
   * A logarithmic knob (a frequency, a time): its bounds, both above 0. A lane's line goes straight along the knob, so in the
   * logarithm of the value (#881, `segmentValue`); absent for a knob linear in its unit, a pan and a volume.
   */
  log?: KnobLog
  /** The value a typed text means, with the bound it was limited to when the text went past the range (#421); null when unreadable. */
  parse: (text: string) => AxisEntry | null
}

/** What a typed text means on an axis: the value (kept to the range and step) and, when the text went past the range, the bound reached. */
export interface AxisEntry {
  value: number
  limit?: number
}

function deviceAxis(descriptor: DeviceDescriptor, param: ParamDescriptor, labels: Labels): Axis {
  return {
    pos: (value) => devicePosition(param, value),
    at: (position) => deviceValueAt(param, position),
    clamp: (value) => snapDevice(param, value),
    min: param.min,
    max: param.max,
    home: devicePosition(param, param.default),
    ...(param.scale === 'log' && param.min > 0 && param.max > param.min ? { log: { min: param.min, max: param.max } } : {}),
    format: (value) => {
      if (isChoice(param)) {
        const key = param.values[Math.round((value - param.min) / (param.step || 1))]
        return key === undefined ? formatValue(param, value) : labels.choice(descriptor, key)
      }
      return formatValue(param, value)
    },
    parse: (text) => parseParamEntry(param, text),
  }
}

const gainAxis = (): Axis => ({
  pos: (db) => faderPosition(db),
  // Silence is not a value of a lane (it stops at the lowest finite gain).
  at: (position) => Math.round((gainAt(position) ?? MIN_GAIN_DB) * 100) / 100,
  clamp: (db) => Math.round(Math.min(MAX_GAIN_DB, Math.max(MIN_GAIN_DB, db)) * 100) / 100,
  min: MIN_GAIN_DB,
  max: MAX_GAIN_DB,
  home: faderPosition(0),
  format: (db) => `${formatGain(db <= MIN_GAIN_DB ? null : db)} dB`,
  parse: (text) => {
    const entry = parseGainEntry(text)
    if (!entry) return null
    const value = entry.gain === null ? MIN_GAIN_DB : entry.gain
    if (entry.limited) return { value, limit: MAX_GAIN_DB }
    // Silence is no value of a lane: a number below its bottom is limited there, the word "-inf" is the bottom itself.
    const q = parseQuantity(text)
    return q && q.value < MIN_GAIN_DB ? { value, limit: MIN_GAIN_DB } : { value }
  },
})

const panAxis = (): Axis => ({
  pos: (pan) => (Math.min(1, Math.max(-1, pan)) + 1) / 2,
  at: (position) => Math.round((Math.min(1, Math.max(0, position)) * 2 - 1) * 100) / 100 + 0,
  clamp: (pan) => Math.round(Math.min(1, Math.max(-1, pan)) * 100) / 100 + 0,
  min: -1,
  max: 1,
  home: 0.5,
  format: formatPan,
  parse: (text) => {
    const entry = parsePanEntry(text)
    if (!entry) return null
    return entry.limited ? { value: entry.pan, limit: entry.pan } : { value: entry.pan }
  },
})

/** The axis of a lane's target on a track, or null when the page cannot draw it (a device without a descriptor). */
export function axisOf(track: Pick<Track, 'devices'>, target: AutomationTarget, labels: Labels): Axis | null {
  if ('mixer' in target) return target.mixer.param === 'gain' ? gainAxis() : panAxis()
  const entry = chainOf(track).find((e) => e.id === target.device.device)
  const descriptor = entry?.descriptor
  const param = descriptor?.params.find((p) => p.key === target.device.param)
  return descriptor && param ? deviceAxis(descriptor, param, labels) : null
}

/** The bounds of a logarithmic knob. */
export interface KnobLog {
  min: number
  max: number
}

/** The logarithmic knob of a lane's target on a track, if its parameter has one (the engine's `Knob`, found the same way). */
export function knobOf(track: Pick<Track, 'devices'>, target: AutomationTarget): KnobLog | undefined {
  if ('mixer' in target) return undefined
  const param = chainOf(track)
    .find((e) => e.id === target.device.device)
    ?.descriptor?.params.find((p) => p.key === target.device.param)
  return param && param.scale === 'log' && param.min > 0 && param.max > param.min ? { min: param.min, max: param.max } : undefined
}

/** One parameter a track can automate. */
export interface Option {
  key: string
  target: AutomationTarget
  /** The group of the menu: "Mixer", or the device's name. */
  group: string
  /** The parameter's name. */
  label: string
}

/** The names of the devices of a chain: the descriptor's name, numbered when a kind appears twice. */
export function deviceNames(chain: readonly ChainEntry[]): Map<string, string> {
  const names = new Map<string, string>()
  const total = new Map<string, number>()
  for (const e of chain) if (e.descriptor) total.set(e.descriptor.kind, (total.get(e.descriptor.kind) ?? 0) + 1)
  const seen = new Map<string, number>()
  for (const e of chain) {
    if (!e.descriptor) continue
    const n = (seen.get(e.descriptor.kind) ?? 0) + 1
    seen.set(e.descriptor.kind, n)
    names.set(e.id, (total.get(e.descriptor.kind) ?? 1) > 1 ? `${e.descriptor.name} ${n}` : e.descriptor.name)
  }
  return names
}

/** Every parameter a track can automate: its volume and pan, then each parameter of each device of its chain that has a descriptor. */
export function optionsOf(track: Pick<Track, 'devices'>, labels: Labels): Option[] {
  const options: Option[] = (['gain', 'pan'] as const).map((param) => ({
    key: targetKey({ mixer: { param } }),
    target: { mixer: { param } },
    group: labels.mixer,
    label: labels.mixerParam(param),
  }))
  const chain = chainOf(track)
  const names = deviceNames(chain)
  for (const e of chain) {
    if (!e.descriptor) continue
    for (const p of e.descriptor.params) {
      const target: AutomationTarget = { device: { device: e.id, param: p.key } }
      options.push({ key: targetKey(target), target, group: names.get(e.id) ?? e.descriptor.name, label: labels.param(e.descriptor, p) })
    }
  }
  return options
}

/** A lane the editor shows. */
export interface LaneInfo {
  id: string
  lane: AutomationLane
  axis: Axis
  /** "Device › parameter", or the mixer's. */
  title: string
  group: string
  label: string
}

/**
 * The lanes of a track the editor can draw, in the order of the menu (mixer first, then the chain). A lane on a device that has
 * no descriptor (a module) is left out: the page cannot name its parameter or range.
 */
export function lanesOf(track: Pick<Track, 'devices' | 'automation'>, labels: Labels): LaneInfo[] {
  const options = optionsOf(track, labels)
  const order = new Map(options.map((o, i) => [o.key, i]))
  const infos: (LaneInfo & { rank: number })[] = []
  for (const [id, lane] of Object.entries(track.automation)) {
    if (!lane) continue
    const key = targetKey(lane.target)
    const option = options.find((o) => o.key === key)
    const axis = option ? axisOf(track, lane.target, labels) : null
    if (!option || !axis) continue
    infos.push({ id, lane, axis, title: `${option.group} › ${option.label}`, group: option.group, label: option.label, rank: order.get(key) ?? 0 })
  }
  infos.sort((a, b) => a.rank - b.rank || (a.id < b.id ? -1 : 1))
  return infos
}

// --- the value of a lane at a tick (the engine's rule, AM-2) -----------------------------------------

/**
 * The value a lane has at `tick`: before the first point the first, after the last the last; between two points a
 * straight line (`linear`, which for a volume lane is a line along the fader's travel, the lane's own axis: the engine interpolates the position of the fader, not the amplitude and not the dB, #389),
 * the earlier value held (`step`), or that line warped in time (`ease_in`, `ease_out`, `smooth`, #881: `segmentValue`). The
 * shape of a point is that of the transition that follows it.
 */
export function valueAt(points: readonly LanePoint[], tick: number, target: AutomationTarget, log?: KnobLog): number | null {
  if (points.length === 0 || !Number.isFinite(tick)) return null
  const next = lowerBound(points, Math.floor(tick) + 1)
  // `next` is the first point after the tick; the one before it is the last point reached.
  if (next === 0) return points[0]!.value
  const a = points[next - 1]!
  const b = points[next]
  if (!b || a.shape === 'step' || b.tick === a.tick) return a.value
  return segmentValue(a, b, (tick - a.tick) / (b.tick - a.tick), target, log)
}

/**
 * The shapes of a point, in the order the interface lists and cycles them (#881): the straight line, the held value, then the
 * three curves of the production sheet (`ease-in`, `ease-out`, `smooth`).
 */
export const SHAPES: readonly Shape[] = ['linear', 'step', 'ease_in', 'ease_out', 'smooth']

/**
 * The progress of a transition of shape `shape` at linear progress `t` in [0, 1] (#881, the engine's `automation::warp`, the
 * same products so that both give the same numbers): `t²` slow then fast, `t (2 − t)` fast then slow, `t² (3 − 2t)` an S; `t`
 * for `linear` (and for `step`, which holds before it gets here).
 */
export function shapeWarp(shape: Shape, t: number): number {
  switch (shape) {
    case 'ease_in':
      return t * t
    case 'ease_out':
      return t * (2 - t)
    case 'smooth':
      return t * t * (3 - 2 * t)
    default:
      return t
  }
}

/**
 * The value at linear progress `t` (0 at `a`, 1 at `b`) of the transition that leaves `a`, which is not a `step`: the straight
 * line along the lane's knob, warped by the shape of `a` (#881): along the fader's travel for a volume lane (#389), in the
 * logarithm of the value for a logarithmic knob (`log`, the founder's decision on #881), in the unit otherwise. The engine's
 * `automation::value_on` and `gain_value`, the same formulas.
 */
export function segmentValue(a: LanePoint, b: LanePoint, t: number, target: AutomationTarget, log?: KnobLog): number {
  const w = shapeWarp(a.shape, Math.min(1, Math.max(0, t)))
  if ('mixer' in target && target.mixer.param === 'gain') {
    const pa = faderPosition(a.value)
    return exactDbAt(pa + (faderPosition(b.value) - pa) * w)
  }
  if (log) {
    const clamp = (v: number) => Math.min(log.max, Math.max(log.min, v))
    const la = Math.log(clamp(a.value))
    const lb = Math.log(clamp(b.value))
    return clamp(Math.exp(la + (lb - la) * w))
  }
  return a.value + (b.value - a.value) * w
}
