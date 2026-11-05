import { describe, expect, it } from 'vitest'
import type { FeedbackReport, RecordedNote } from '../engine/feedback'
import type { Command, ProjectEvent, ViewSlots } from '../generated/protocol'
import { GIVE_UP_MS, Recorder, SETTLE_MS } from './recorder.svelte'

const SLOTS: ViewSlots = { epoch: 1, tracks: { a: 0, b: 1 }, scenes: { s1: 0, s2: 1 } }

function rig(applies = true, mode: 'web' | 'native' = 'web') {
  let now = 0
  const ran: Command[] = []
  const recorder = new Recorder({
    run: (command) => {
      ran.push(command)
      return applies || command.type !== 'record.commit_take'
    },
    slots: () => SLOTS,
    now: () => now,
    mode: () => mode,
  })
  return {
    recorder,
    ran,
    at: (ms: number) => (now = ms),
  }
}

function report(over: Partial<FeedbackReport>): FeedbackReport {
  return {
    seq: 2,
    epoch: 1,
    transport: 'recording',
    playheadTick: 0,
    playheadFrame: 0,
    cpuLoad: null,
    cpuPeak: null,
    liveOverflows: 0,
    rejected: 0,
    diskUnderruns: 0,
    offlineSources: 0,
    pcmOverBudget: false,
    pcmKib: 0,
    silencedFrames: 0,
    voices: 0,
    tracks: [],
    master: { peakL: 0, peakR: 0, rmsL: 0, rmsR: 0, gainReductionDb: 0 },
    clips: [],
    held: [],
    recordTake: 0,
    recordTotal: 0,
    recordDropped: 0,
    recordCapturing: false,
    recorded: [],
    ...over,
  }
}

const note = (index: number, track: number, start: number, pitch = 60): RecordedNote => ({ index, track, start, length: 240, pitch, velocity: 100 })

const changed = (take: number, state: 'recording' | 'stopped' | 'committed' | 'discarded'): ProjectEvent => ({ type: 'recording_changed', take, state })

function start(r: ReturnType<typeof rig>, target: { at: 'scene'; scene: string } | { at: 'arrangement' } = { at: 'arrangement' }) {
  r.recorder.ran({ type: 'record.start', target }, { take: 1, target, tracks: ['a'], shift_ms: 25, count_in: true }, [changed(1, 'recording')])
}

describe('Recorder', () => {
  it('gathers the notes of its engine take once each, then commits them after the engine ends the capture', () => {
    const r = rig()
    // The engine's last take before this one was 4.
    r.recorder.feedback(report({ recordTake: 4 }))
    start(r)
    expect(r.recorder.phase).toBe('recording')
    // A report of the previous take: ignored.
    r.recorder.feedback(report({ recordTake: 4, recordTotal: 1, recorded: [note(0, 0, 999)] }))
    r.recorder.feedback(report({ recordTake: 5, recordCapturing: true, recordTotal: 1, recorded: [note(0, 0, 100)] }))
    // The same note again, then a new one.
    r.recorder.feedback(report({ recordTake: 5, recordCapturing: true, recordTotal: 3, recorded: [note(0, 0, 100), note(1, 0, 400, 62)] }))
    expect(r.recorder.notes.a?.map((n) => n.start)).toEqual([100, 400])
    r.at(1000)
    r.recorder.ran({ type: 'record.stop' }, { take: 1 }, [changed(1, 'stopped')])
    expect(r.recorder.phase).toBe('closing')
    // Too early, and still capturing: nothing yet.
    r.at(1000 + SETTLE_MS - 1)
    r.recorder.feedback(report({ recordTake: 5, recordCapturing: false, recordTotal: 3 }))
    r.at(1000 + SETTLE_MS)
    r.recorder.feedback(report({ recordTake: 5, recordCapturing: true, recordTotal: 3 }))
    expect(r.ran).toEqual([])
    r.recorder.feedback(report({ recordTake: 5, recordCapturing: false, recordTotal: 4, recorded: [note(3, 0, 800, 64)] }))
    expect(r.ran).toEqual([
      {
        type: 'record.commit_take',
        take: 1,
        target: { at: 'arrangement' },
        // The engine moved the notes by the take's shift (25 ms) already (#560).
        shift_ms: 0,
        tracks: [
          {
            track: 'a',
            anchor: null,
            notes: [
              { start: 100, length: 240, pitch: 60, velocity: 100 },
              { start: 400, length: 240, pitch: 62, velocity: 100 },
              { start: 800, length: 240, pitch: 64, velocity: 100 },
            ],
          },
        ],
      },
    ])
    expect(r.recorder.phase).toBe('idle')
    expect(r.recorder.lost).toBe(0)
  })

  it('discards a take with no note, also when the engine never started it', () => {
    const r = rig()
    start(r)
    r.at(10)
    r.recorder.ran({ type: 'record.stop' }, { take: 1 }, [changed(1, 'stopped')])
    r.at(10 + SETTLE_MS)
    r.recorder.feedback(report({ recordTake: 0 }))
    expect(r.ran).toEqual([{ type: 'record.discard_take' }])
  })

  it('counts the notes it missed and gives up waiting after a while', () => {
    const r = rig()
    start(r)
    r.recorder.feedback(report({ recordTake: 1, recordCapturing: true, recordTotal: 300, recordDropped: 0, recorded: [note(299, 0, 10)] }))
    expect(r.recorder.lost).toBe(299)
    r.recorder.ran({ type: 'transport.stop' }, {}, [changed(1, 'stopped')])
    r.at(GIVE_UP_MS)
    r.recorder.feedback(report({ recordTake: 1, recordCapturing: true, recordTotal: 300 }))
    expect(r.ran.map((c) => c.type)).toEqual(['record.commit_take'])
  })

  it('keeps the anchor of the target cell while its clip plays', () => {
    const r = rig()
    start(r, { at: 'scene', scene: 's2' })
    r.recorder.feedback(
      report({
        recordTake: 1,
        recordCapturing: true,
        recordTotal: 1,
        recorded: [note(0, 0, 5000)],
        clips: [
          { track: 0, scene: 0, state: 'playing', anchor: 1 },
          { track: 0, scene: 1, state: 'playing', anchor: 3840 },
        ],
      }),
    )
    r.recorder.ran({ type: 'record.stop' }, { take: 1 }, [changed(1, 'stopped')])
    r.at(SETTLE_MS)
    r.recorder.feedback(report({ recordTake: 1, recordTotal: 1 }))
    const commit = r.ran[0] as Extract<Command, { type: 'record.commit_take' }>
    expect(commit.tracks[0]!.anchor).toBe(3840)
  })

  it('drops the take when the engine is replaced, and frees the project when it refuses the take', () => {
    const r = rig()
    start(r)
    r.recorder.engineReplaced()
    expect(r.recorder.take).toBeNull()
    expect(r.recorder.dropped).toBe('engine')
    const refused = rig(false)
    start(refused)
    refused.recorder.feedback(report({ recordTake: 1, recordCapturing: true, recordTotal: 1, recorded: [note(0, 0, 0)] }))
    refused.recorder.ran({ type: 'record.stop' }, { take: 1 }, [changed(1, 'stopped')])
    refused.at(SETTLE_MS)
    refused.recorder.feedback(report({ recordTake: 1, recordTotal: 1 }))
    expect(refused.ran.map((c) => c.type)).toEqual(['record.commit_take', 'record.discard_take'])
    expect(refused.recorder.dropped).toBe('refused')
  })

  it('turns an undo during the take into a discard, and leaves it alone otherwise', () => {
    const r = rig()
    expect(r.recorder.instead({ type: 'history.undo' })).toBeNull()
    start(r)
    expect(r.recorder.instead({ type: 'history.undo' })).toEqual({ type: 'record.discard_take' })
    expect(r.recorder.instead({ type: 'history.redo' })).toBeNull()
    // Waiting to be written: an undo drops it too, it never reaches an older entry.
    r.recorder.ran({ type: 'record.stop' }, { take: 1 }, [changed(1, 'stopped')])
    expect(r.recorder.instead({ type: 'history.undo' })).toEqual({ type: 'record.discard_take' })
    r.recorder.ran({ type: 'record.discard_take' }, { take: 1 }, [changed(1, 'discarded')])
    expect(r.recorder.instead({ type: 'history.undo' })).toBeNull()
  })

  it('keeps the notes of a track armed during the take', () => {
    const r = rig()
    start(r)
    r.recorder.feedback(report({ recordTake: 1, recordCapturing: true, recordTotal: 2, recorded: [note(0, 0, 10), note(1, 1, 20)] }))
    expect(Object.keys(r.recorder.notes).sort()).toEqual(['a', 'b'])
  })

  it('after a new engine, takes the first engine take it sees capturing, not an old one still in a report', () => {
    const r = rig()
    r.recorder.feedback(report({ recordTake: 7 }))
    r.recorder.engineReplaced()
    start(r)
    // A report of the old take, not capturing: not this take.
    r.recorder.feedback(report({ recordTake: 7, recordTotal: 1, recorded: [note(0, 0, 999)] }))
    expect(r.recorder.notes).toEqual({})
    r.recorder.feedback(report({ recordTake: 8, recordCapturing: true, recordTotal: 1, recorded: [note(0, 0, 5)] }))
    expect(r.recorder.notes.a?.map((n) => n.start)).toEqual([5])
  })

  it('drops the take in the project when the engine is left for Boost, and keeps the anchor of a track armed later', () => {
    const r = rig()
    start(r, { at: 'scene', scene: 's1' })
    r.recorder.feedback(report({ recordTake: 1, recordCapturing: true, clips: [{ track: 1, scene: 0, state: 'playing', anchor: 960 }] }))
    r.recorder.feedback(report({ recordTake: 1, recordCapturing: true, recordTotal: 1, recorded: [note(0, 1, 2000)] }))
    r.recorder.ran({ type: 'record.stop' }, { take: 1 }, [changed(1, 'stopped')])
    r.at(SETTLE_MS)
    r.recorder.feedback(report({ recordTake: 1, recordTotal: 1 }))
    const commit = r.ran[0] as Extract<Command, { type: 'record.commit_take' }>
    expect(commit.tracks).toEqual([{ track: 'b', anchor: 960, notes: [{ start: 2000, length: 240, pitch: 60, velocity: 100 }] }])
    const left = rig()
    start(left)
    left.recorder.abandon()
    expect(left.ran).toEqual([{ type: 'record.discard_take' }])
    expect(left.recorder.phase).toBe('idle')
    expect(left.recorder.dropped).toBe('engine')
    left.recorder.abandon()
    expect(left.ran).toHaveLength(1)
  })

  it('refuses a take in Boost, and counts the notes the commit or a full live queue lost', () => {
    expect(rig(true, 'native').recorder.refusal({ type: 'record.start' })).toBe('transport.recordBoost')
    expect(rig(true, 'native').recorder.refusal({ type: 'record.stop' })).toBeNull()
    expect(rig().recorder.refusal({ type: 'record.start' })).toBeNull()
    const r = rig()
    r.recorder.feedback(report({ liveOverflows: 3 }))
    start(r)
    r.recorder.feedback(report({ recordTake: 1, recordCapturing: true, liveOverflows: 3 }))
    r.recorder.feedback(report({ recordTake: 1, recordCapturing: true, liveOverflows: 5, recordTotal: 1, recorded: [note(0, 0, 0)] }))
    expect(r.recorder.lost).toBe(2)
    r.recorder.ran({ type: 'record.commit_take', take: 1, target: { at: 'arrangement' }, tracks: [] }, { clips: [], dropped: 4 }, [])
    expect(r.recorder.lost).toBe(6)
  })

  it('ends when another author discards or commits the take', () => {
    const r = rig()
    start(r)
    r.recorder.ran({ type: 'record.discard_take' }, { take: 1 }, [changed(1, 'discarded')])
    expect(r.recorder.phase).toBe('idle')
    expect(r.ran).toEqual([])
  })

  it('never holds a take for ever when the engine reports another epoch: it waits, then drops it, in the project too (#513)', () => {
    const r = rig()
    start(r)
    r.recorder.feedback(report({ recordTake: 1, recordCapturing: true, recordTotal: 1, recorded: [note(0, 0, 10)] }))
    r.at(10)
    r.recorder.ran({ type: 'record.stop' }, { take: 1 }, [changed(1, 'stopped')])
    // Reports of another epoch: nothing is gathered, nothing is written, the take waits for a matching one...
    r.at(10 + SETTLE_MS)
    r.recorder.feedback(report({ epoch: 2, recordTake: 1, recordTotal: 2, recorded: [note(1, 0, 20)] }))
    r.at(10 + GIVE_UP_MS - 1)
    r.recorder.feedback(report({ epoch: 2, recordTake: 1 }))
    expect(r.recorder.phase).toBe('closing')
    expect(r.ran).toEqual([])
    // ... a matching one ends it, and the note of the other epoch is not in the take.
    r.recorder.feedback(report({ recordTake: 1, recordTotal: 1 }))
    expect(r.ran.map((c) => c.type)).toEqual(['record.commit_take'])
    expect(JSON.stringify(r.ran[0])).not.toContain('"start":20')
    expect(r.recorder.phase).toBe('idle')

    // No matching report ever comes: past the give-up the take is dropped and said, not held.
    const stuck = rig()
    start(stuck)
    stuck.recorder.ran({ type: 'record.stop' }, { take: 1 }, [changed(1, 'stopped')])
    stuck.at(GIVE_UP_MS - 1)
    stuck.recorder.feedback(report({ epoch: 2 }))
    expect(stuck.recorder.phase).toBe('closing')
    stuck.at(GIVE_UP_MS)
    stuck.recorder.feedback(report({ epoch: 2 }))
    expect(stuck.recorder.phase).toBe('idle')
    expect(stuck.recorder.dropped).toBe('engine')
    expect(stuck.ran).toEqual([{ type: 'record.discard_take' }])

    // A take still recording is left alone by a report of another epoch, however late.
    const live = rig()
    start(live)
    live.at(10 * GIVE_UP_MS)
    live.recorder.feedback(report({ epoch: 2 }))
    expect(live.recorder.phase).toBe('recording')
    expect(live.ran).toEqual([])
    // And a new engine frees a take waiting to be written at once.
    live.recorder.ran({ type: 'record.stop' }, { take: 1 }, [changed(1, 'stopped')])
    live.recorder.engineReplaced()
    expect(live.recorder.phase).toBe('idle')
  })
})
