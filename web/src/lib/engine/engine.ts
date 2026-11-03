import type { Command, MidiInput } from '../generated/protocol'
import type { FeedbackReport } from './feedback'

export type EngineKind = 'web' | 'native'

export interface EngineStatus {
  kind: EngineKind
  sampleRate: number
  /** Estimated output latency in ms, from what the browser or driver reports; never measured. */
  estimatedLatencyMs: number | null
}

/** What an engine tells the page. */
export interface EngineEvents {
  onStatus(status: EngineStatus): void
  /** The engine's report, about 30 times per second (both modes decode the same bytes). */
  onFeedback(report: FeedbackReport): void
  /** Native engine only: its MIDI inputs (the page owns MIDI in Web mode). */
  onMidiInputs?(inputs: MidiInput[], selected: string | null): void
  onMidiActivity?(note: number, velocity: number): void
  onError(code: string): void
  /**
   * The engine stopped taking the message stream (`MsgError::code`: out of memory, engine busy,
   * corrupt stream) at byte `at` of a push: it is behind the project until it is replaced by a new
   * engine that starts from a snapshot (the dispatcher's restart).
   */
  onStreamRefused?(code: number, at: number): void
  /** The engine stopped on its own (helper gone, session replaced). */
  onClosed(): void
}

export interface Engine {
  readonly kind: EngineKind
  /**
   * Hands the engine what a command the project applied means for it: the Web engine takes the
   * engine bytes the page's project core encoded (it ignores the command), the helper's engine
   * takes the command (its temporary copy of the project, until BO-1, ignores the bytes).
   */
  send(command: Command, bytes: Uint8Array): void
  /**
   * The frame of the engine's host clock at which a note was played at `timeStamp` (the page's
   * performance clock), for `live.input_note_on/off` (#499); `null` when the engine cannot tell.
   */
  frameAt?(timeStamp: number | undefined): number | null
  close(): Promise<void>
}
