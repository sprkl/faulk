/**
 * The page's questions to the user before an agent's call that cannot be undone runs (AG-4, #100): an `x-destructive`
 * command (`DESTRUCTIVE_COMMANDS`), or a storage action that replaces or deletes a project or settles a conflict
 * (`AGENT_CONFIRMED` in `dispatcher.ts`). One question is shown at a time, in the order the calls came; each has
 * the deadline its call was given when it reached the page (`AGENT_CONFIRM_MS` later), so that the agent always
 * learns the answer before the helper stops waiting for the page.
 *
 * Each question is answered once: `confirmed` (the user allowed it), `declined`, `timeout` (the deadline passed,
 * whether its timer fired or the user answered too late, as in a hidden tab whose timers the browser slows down),
 * or `withdrawn` (the link that carried the call closed: the agent was told `page.gone`, so nothing may run). Only
 * `confirmed` lets the call run.
 */

/** How a question was answered. */
export type ConfirmAnswer = 'confirmed' | 'declined' | 'timeout' | 'withdrawn'

/** What the agent asks to do; `target` is the project it is about, when the page knows its name. */
export type AgentQuestion =
  | { kind: 'new' }
  | { kind: 'open'; target: string | null }
  | { kind: 'delete'; target: string | null }
  | { kind: 'keepMine'; target: string | null }
  | { kind: 'loadFolder'; target: string | null }
  /** Any other destructive command, named. */
  | { kind: 'command'; command: string }

/** A question waiting for the user. */
export interface PendingQuestion {
  id: number
  /** The history's author of the agent: `agent:<name>`. */
  agent: string
  question: AgentQuestion
  /** On the queue's clock: past it, the question is answered `timeout`. */
  deadline: number
}

/** The time and timers the queue uses (tests move them by hand). */
export interface ConfirmClock {
  now(): number
  setTimeout(run: () => void, ms: number): unknown
  clearTimeout(handle: unknown): void
}

const browserClock: ConfirmClock = {
  now: () => Date.now(),
  setTimeout: (run, ms) => setTimeout(run, ms),
  clearTimeout: (handle) => clearTimeout(handle as ReturnType<typeof setTimeout>),
}

interface Waiting extends PendingQuestion {
  resolve(answer: ConfirmAnswer): void
}

export class AgentConfirmations {
  /** The questions waiting, the one shown first. */
  pending = $state.raw<PendingQuestion[]>([])
  private waiting: Waiting[] = []
  private nextId = 1

  constructor(readonly clock: ConfirmClock = browserClock) {}

  /** The question shown, or `null`. */
  get current(): PendingQuestion | null {
    return this.pending[0] ?? null
  }

  /**
   * Asks the user whether `agent` may do `question`, until `deadline` (on `clock`). `signal` is the call's link:
   * when it aborts, the question is withdrawn.
   */
  ask(agent: string, question: AgentQuestion, deadline: number, signal?: AbortSignal): Promise<ConfirmAnswer> {
    if (signal?.aborted) return Promise.resolve('withdrawn')
    const left = deadline - this.clock.now()
    if (left <= 0) return Promise.resolve('timeout')
    return new Promise((resolve) => {
      const id = this.nextId++
      let timer: unknown = null
      const onAbort = () => end('withdrawn')
      const end = (answer: ConfirmAnswer) => {
        if (!this.waiting.some((w) => w.id === id)) return
        this.clock.clearTimeout(timer)
        signal?.removeEventListener('abort', onAbort)
        this.waiting = this.waiting.filter((w) => w.id !== id)
        this.publish()
        resolve(answer)
      }
      this.waiting = [...this.waiting, { id, agent, question, deadline, resolve: end }]
      timer = this.clock.setTimeout(() => end('timeout'), left)
      signal?.addEventListener('abort', onAbort, { once: true })
      this.publish()
    })
  }

  /** The user's answer to the question shown: allow it (`true`) or refuse it. Past its deadline, it is the timeout. */
  answer(allow: boolean) {
    const head = this.waiting[0]
    if (!head) return
    if (this.clock.now() >= head.deadline) head.resolve('timeout')
    else head.resolve(allow ? 'confirmed' : 'declined')
  }

  private publish() {
    this.pending = this.waiting.map(({ id, agent, question, deadline }) => ({ id, agent, question, deadline }))
  }
}
