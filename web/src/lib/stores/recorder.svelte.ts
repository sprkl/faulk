import type { FeedbackReport, RecordedNote } from '../engine/feedback'
import type { Command, ProjectEvent, RecordTarget, ViewSlots } from '../generated/protocol'

/**
 * MIDI recording on the page's side (#470, `docs/specs/03-enregistrement-midi.md`): the page owns the project, the
 * engine stamps the notes. This store follows a take from `record.start` to its end, gathers the notes the engine
 * returns in its reports (by their number in the engine's take, so a report read twice or missed loses nothing until
 * the page falls 256 notes behind, which it counts), and once the capture has ended and the engine says so (its
 * capture flag is off), writes the take with `record.commit_take`, one history entry, or drops it with
 * `record.discard_take` when nothing was played. It runs commands through the dispatcher like any view.
 */

/** A take as `record.start` answered. */
export interface Take {
  take: number
  target: RecordTarget
  /** The recording tracks (ids). */
  tracks: string[]
  countIn: boolean
}

/** How long after the end of the capture the page waits at least, so that the report it reads was written after it (ms). */
export const SETTLE_MS = 100
/** How long the page waits at most for the engine to confirm the end of a take before it writes what it has (ms). */
export const GIVE_UP_MS = 2000

/** Where the store is. */
export type RecorderPhase = 'idle' | 'recording' | 'closing'

export interface RecorderPorts {
  /** Runs a command; whether the project applied it. */
  run(command: Command): boolean
  /** Which engine plays: recording works with the Web engine only, until the helper holds the page's project (#501). */
  mode(): 'web' | 'native'
  /** The engine slots of the project's tracks and scenes. */
  slots(): ViewSlots | null
  now(): number
}

/* eslint-disable svelte/prefer-svelte-reactivity -- the gathered notes and anchors are not reactive on purpose: the views read `notes`, replaced whole */
export class Recorder {
  take = $state<Take | null>(null)
  phase = $state<RecorderPhase>('idle')
  /** The notes of the take so far, by track id, as played (transport ticks), for the views that show them live. */
  notes = $state.raw<Readonly<Record<string, readonly RecordedNote[]>>>({})
  /** Notes the take lost: the engine's held table was full, or the page fell behind its reports. */
  lost = $state(0)
  /** Why the last take was dropped without being written, if it was: the engine was replaced, or the project refused it. */
  dropped = $state<'engine' | 'refused' | null>(null)

  /** The engine's take number before this take (`null`: no report of this engine read yet). */
  private baseline: number | null = 0
  private engineTake: number | null = null
  /** The next note number the page expects. */
  private next = 0
  private collected = new Map<number, RecordedNote[]>()
  /** Per recording track id: the transport tick of position 0 of the target cell's clip, while it played. */
  private anchors = new Map<string, number>()
  private endedAt = 0
  /** Notes the page missed between two reports it read. */
  private gaps = 0
  /** The engine's count of live events lost to a full live queue when the take started (`null`: no report yet). */
  private overflowBase: number | null = null
  /** Live events lost to a full live queue during the take: notes the engine never received. */
  private overflowed = 0
  /** The engine's take number in the last report, `null` before the first report of a new engine. */
  private lastTake: number | null = 0

  constructor(private readonly ports: RecorderPorts) {}

  /**
   * The command to run instead of `command`, if any: an undo during a take, or while it waits to be written, drops the
   * take (spec § 7): it wrote nothing yet, so the undo never reaches an older entry meanwhile.
   */
  instead(command: Command): Command | null {
    return command.type === 'history.undo' && this.phase !== 'idle' ? { type: 'record.discard_take' } : null
  }

  /**
   * Why `command` is refused before it reaches the project, if it is: a take does not start in Boost (#501), whoever
   * asks (the button is disabled there, an agent's `record.start` is refused here with the same reason).
   */
  refusal(command: Command): 'transport.recordBoost' | null {
    return command.type === 'record.start' && this.ports.mode() === 'native' ? 'transport.recordBoost' : null
  }

  /** A command the dispatcher ran, with its output and its events. */
  ran(command: Command, output: unknown, events: readonly ProjectEvent[]) {
    // Notes the project could not place (before tick 0, before their clip): lost too, and said.
    if (command.type === 'record.commit_take' && output && typeof output === 'object' && 'dropped' in output) {
      const dropped = Number((output as { dropped: number }).dropped) || 0
      if (dropped > 0) this.lost += dropped
    }
    if (command.type === 'record.start' && output && typeof output === 'object' && 'take' in output) {
      const o = output as { take: number; target: RecordTarget; tracks: string[]; count_in: boolean }
      this.begin({ take: o.take, target: o.target, tracks: o.tracks, countIn: o.count_in })
    }
    for (const event of events) {
      if (event.type !== 'recording_changed' || event.take !== this.take?.take) continue
      if (event.state === 'stopped') {
        this.phase = 'closing'
        this.endedAt = this.ports.now()
      } else if (event.state === 'committed' || event.state === 'discarded') {
        this.reset()
      }
    }
  }

  private begin(take: Take) {
    this.take = take
    this.phase = 'recording'
    this.baseline = this.lastTake
    this.engineTake = null
    this.next = 0
    this.collected = new Map()
    this.anchors = new Map()
    this.notes = {}
    this.lost = 0
    this.gaps = 0
    this.overflowBase = null
    this.overflowed = 0
    this.dropped = null
  }

  private reset() {
    this.take = null
    this.phase = 'idle'
    this.engineTake = null
    this.collected = new Map()
    this.anchors = new Map()
    this.notes = {}
  }

  /** The engine was replaced (a restart, a switch of mode): it holds the take no more, and the core forgot it. */
  engineReplaced() {
    if (this.take) this.dropped = 'engine'
    this.reset()
    this.lastTake = null
  }

  /**
   * The engine that captures the take is about to be left (a switch to Boost): the take is dropped, in the project too
   * (`record.discard_take`), and said, so that the next take can start on the new engine. Also a take that waited
   * `GIVE_UP_MS` to be written while the engine reports another slot table than the project's.
   */
  abandon() {
    if (!this.take) return
    this.ports.run({ type: 'record.discard_take' })
    this.reset()
    this.dropped = 'engine'
    this.lastTake = null
  }

  /** A report of the engine. */
  feedback(report: FeedbackReport) {
    this.lastTake = report.recordTake
    const take = this.take
    if (!take) return
    const slots = this.ports.slots()
    if (slots && report.epoch !== slots.epoch) {
      // The engine reports another slot table than the project's: its notes cannot be named, and the give-up below
      // would never be reached. A take left waiting this long is dropped, in the project too, rather than held for ever (#513).
      // Written, it would be wrong: the notes gathered are named by the engine's slots, and the project's slots are no longer those.
      if (this.phase === 'closing' && this.ports.now() - this.endedAt >= GIVE_UP_MS) this.abandon()
      return
    }
    // This take is the first engine take after the baseline; without a baseline (no report read since the engine was
    // replaced), the first one seen capturing.
    if (this.engineTake === null && (this.baseline === null ? report.recordCapturing : report.recordTake > this.baseline)) this.engineTake = report.recordTake
    if (this.overflowBase === null) this.overflowBase = report.liveOverflows
    else if (report.liveOverflows > this.overflowBase) {
      this.overflowed = report.liveOverflows - this.overflowBase
      this.lost = this.gaps + report.recordDropped + this.overflowed
    }
    if (this.engineTake !== null && report.recordTake === this.engineTake) this.gather(report, slots)
    this.watchAnchors(report, take, slots)
    if (this.phase !== 'closing') return
    const waited = this.ports.now() - this.endedAt
    if (waited < SETTLE_MS) return
    const settled = !report.recordCapturing && (this.engineTake === null || report.recordTake === this.engineTake)
    if (settled || waited >= GIVE_UP_MS) this.finish(take)
  }

  private gather(report: FeedbackReport, slots: ViewSlots | null) {
    const first = report.recorded[0]?.index ?? report.recordTotal
    // Notes the page never saw: the report no longer carries them.
    if (first > this.next) this.gaps += first - this.next
    let changed = false
    for (const note of report.recorded) {
      if (note.index < this.next) continue
      const list = this.collected.get(note.track) ?? []
      list.push(note)
      this.collected.set(note.track, list)
      changed = true
    }
    this.next = Math.max(this.next, report.recordTotal)
    this.lost = this.gaps + report.recordDropped + this.overflowed
    if (changed) this.notes = this.byTrack(slots)
  }

  /**
   * The notes gathered so far, by track id: every track the engine recorded, a track armed during the take included
   * (the engine records the tracks armed when each note starts).
   */
  private byTrack(slots: ViewSlots | null): Record<string, RecordedNote[]> {
    const out: Record<string, RecordedNote[]> = {}
    if (!slots || !this.take) return out
    for (const [id, slot] of Object.entries(slots.tracks)) {
      const notes = slot === undefined ? undefined : this.collected.get(slot)
      if (notes?.length) out[id] = notes
    }
    return out
  }

  private watchAnchors(report: FeedbackReport, take: Take, slots: ViewSlots | null) {
    if (take.target.at !== 'scene' || !slots) return
    const scene = slots.scenes[take.target.scene]
    if (scene === undefined) return
    // Every track: one armed during the take records too.
    for (const [id, track] of Object.entries(slots.tracks)) {
      const cell = report.clips.find((c) => c.track === track && c.scene === scene && (c.state === 'playing' || c.state === 'stopping'))
      if (cell) this.anchors.set(id, cell.anchor)
    }
  }

  private finish(take: Take) {
    const notes = this.byTrack(this.ports.slots())
    const tracks = Object.keys(notes)
      .sort()
      .map((id) => ({
        track: id,
        notes: notes[id]!.map((n) => ({ start: n.start, length: Math.max(1, n.length), pitch: n.pitch, velocity: Math.max(1, n.velocity) })),
        anchor: this.anchors.get(id) ?? null,
      }))
    // The take is over for the page whatever the command says: a refusal is shown by the dispatcher.
    const target = take.target
    this.reset()
    if (tracks.length === 0) {
      this.ports.run({ type: 'record.discard_take' })
      // The engine already moved the notes by the take's shift, on its own clock (#560): no shift here.
    } else if (!this.ports.run({ type: 'record.commit_take', take: take.take, target, shift_ms: 0, tracks })) {
      // Refused (a limit of the project): the dispatcher said why; the take is dropped so the next one may start.
      this.dropped = 'refused'
      this.ports.run({ type: 'record.discard_take' })
    }
  }
}
