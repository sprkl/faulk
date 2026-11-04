import { describe, expect, it } from 'vitest'
import { AgentConfirmations, type ConfirmClock } from './confirm.svelte'

/** A clock the test moves by hand: timers fire only when `advance` passes them, or never (`late`). */
function fakeClock(late = false) {
  let now = 1000
  let next = 1
  const timers = new Map<number, { at: number; run: () => void }>()
  const clock: ConfirmClock = {
    now: () => now,
    setTimeout: (run, ms) => {
      const id = next++
      timers.set(id, { at: now + ms, run })
      return id
    },
    clearTimeout: (id) => void timers.delete(id as number),
  }
  return {
    clock,
    timers,
    advance(ms: number) {
      now += ms
      if (late) return
      for (const [id, timer] of [...timers]) {
        if (timer.at <= now) {
          timers.delete(id)
          timer.run()
        }
      }
    },
  }
}

const settle = () => new Promise((resolve) => setTimeout(resolve, 0))

describe('AgentConfirmations', () => {
  it('shows a question until the user allows it', async () => {
    const { clock } = fakeClock()
    const queue = new AgentConfirmations(clock)
    const answer = queue.ask('agent:a', { kind: 'new' }, 1000 + 45_000)
    expect(queue.current).toMatchObject({ agent: 'agent:a', question: { kind: 'new' }, deadline: 46_000 })
    queue.answer(true)
    expect(await answer).toBe('confirmed')
    expect(queue.current).toBeNull()
  })

  it('refuses when the user refuses', async () => {
    const queue = new AgentConfirmations(fakeClock().clock)
    const answer = queue.ask('agent:a', { kind: 'command', command: 'project.open' }, 46_000)
    queue.answer(false)
    expect(await answer).toBe('declined')
    expect(queue.pending).toEqual([])
  })

  it('answers the timeout at the deadline and drops the question', async () => {
    const fake = fakeClock()
    const queue = new AgentConfirmations(fake.clock)
    const answer = queue.ask('agent:a', { kind: 'new' }, 46_000)
    fake.advance(44_999)
    expect(queue.current).not.toBeNull()
    fake.advance(1)
    expect(await answer).toBe('timeout')
    expect(queue.current).toBeNull()
    expect(fake.timers.size).toBe(0)
  })

  it('counts an answer given after the deadline as the timeout, even when the timer is late', async () => {
    // A hidden tab throttles timers: the deadline still holds when the user answers.
    const fake = fakeClock(true)
    const queue = new AgentConfirmations(fake.clock)
    const answer = queue.ask('agent:a', { kind: 'new' }, 46_000)
    fake.advance(45_000)
    queue.answer(true)
    expect(await answer).toBe('timeout')
    expect(queue.current).toBeNull()
  })

  it('asks a question whose deadline already passed as the timeout, without showing it', async () => {
    const queue = new AgentConfirmations(fakeClock().clock)
    expect(await queue.ask('agent:a', { kind: 'new' }, 1000)).toBe('timeout')
    expect(queue.pending).toEqual([])
  })

  it('keeps questions in order, one at a time, each with its own deadline', async () => {
    const fake = fakeClock()
    const queue = new AgentConfirmations(fake.clock)
    const first = queue.ask('agent:a', { kind: 'new' }, 46_000)
    const second = queue.ask('agent:b', { kind: 'delete', target: 'Sketch' }, 21_000)
    const third = queue.ask('agent:c', { kind: 'keepMine', target: 'Song' }, 50_000)
    expect(queue.current?.agent).toBe('agent:a')
    expect(queue.pending).toHaveLength(3)
    // The second times out while it waits behind the first: it leaves the queue unseen.
    fake.advance(20_000)
    expect(await second).toBe('timeout')
    expect(queue.pending.map((p) => p.agent)).toEqual(['agent:a', 'agent:c'])
    queue.answer(false)
    expect(await first).toBe('declined')
    expect(queue.current?.agent).toBe('agent:c')
    queue.answer(true)
    expect(await third).toBe('confirmed')
  })

  it('ignores an answer when no question is open', () => {
    const queue = new AgentConfirmations(fakeClock().clock)
    expect(() => queue.answer(true)).not.toThrow()
    expect(queue.current).toBeNull()
  })

  it('withdraws a question whose call can no longer be answered (its link closed)', async () => {
    const fake = fakeClock()
    const queue = new AgentConfirmations(fake.clock)
    const link = new AbortController()
    const withdrawn = queue.ask('agent:a', { kind: 'new' }, 46_000, link.signal)
    const kept = queue.ask('agent:b', { kind: 'new' }, 46_000)
    link.abort()
    expect(await withdrawn).toBe('withdrawn')
    expect(queue.current?.agent).toBe('agent:b')
    // An answer now goes to the question still open, never to the withdrawn one.
    queue.answer(true)
    expect(await kept).toBe('confirmed')
    expect(fake.timers.size).toBe(0)
  })

  it("withdraws the questions of an agent that left, and only that agent's (#641)", async () => {
    const fake = fakeClock()
    const queue = new AgentConfirmations(fake.clock)
    const first = queue.ask('agent:a', { kind: 'new' }, 46_000)
    const other = queue.ask('agent:b', { kind: 'new' }, 46_000)
    const second = queue.ask('agent:a', { kind: 'command', command: 'track.delete' }, 46_000)
    queue.withdraw('agent:a')
    expect(await first).toBe('withdrawn')
    expect(await second).toBe('withdrawn')
    expect(queue.pending.map((p) => p.agent)).toEqual(['agent:b'])
    queue.withdraw('agent:nobody')
    queue.answer(true)
    expect(await other).toBe('confirmed')
    expect(fake.timers.size).toBe(0)
  })

  it('withdraws at once a question whose link closed before it was asked', async () => {
    const link = new AbortController()
    link.abort()
    const queue = new AgentConfirmations(fakeClock().clock)
    expect(await queue.ask('agent:a', { kind: 'new' }, 46_000, link.signal)).toBe('withdrawn')
    expect(queue.pending).toEqual([])
  })

  it('answers each question once, whatever comes after', async () => {
    const fake = fakeClock()
    const queue = new AgentConfirmations(fake.clock)
    const link = new AbortController()
    const answer = queue.ask('agent:a', { kind: 'new' }, 46_000, link.signal)
    queue.answer(true)
    link.abort()
    fake.advance(60_000)
    await settle()
    expect(await answer).toBe('confirmed')
  })
})
