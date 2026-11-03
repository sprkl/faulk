import type { Command, HelperUpdate, MidiInput } from '../generated/protocol'
import type { FeedbackReport } from './feedback'

export type EngineKind = 'web' | 'native'

/** The helper's self-update as its last `update.status` told it (#515). */
export interface HelperUpdateState {
  /** The installed version; `null` when the helper is not installed (run from `cargo`). */
  current: string | null
  /** End of the helper's last check, in seconds since the Unix epoch; `null` before the first. */
  checkedAt: number | null
  status: HelperUpdate
}

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
  /**
   * Native engine only: a message of the helper's selected MIDI input, as received (BO-1). The page
   * plays it as a Web MIDI message: the helper holds no project and routes no note.
   */
  onMidiMessage?(data: number[]): void
  /** Native engine only: the helper's self-update (`update.status`), at session start and on change. */
  onHelperUpdate?(update: HelperUpdateState): void
  onError(code: string): void
  /**
   * The engine stopped taking the message stream (`MsgError::code`: out of memory, engine busy,
   * corrupt stream; for the helper also a stream not of its engine's epoch) at byte `at` of a
   * push: it is behind the project. The Web engine is replaced by a new one that starts from a
   * snapshot (the dispatcher's restart); the helper's gets a snapshot in an epoch newer than
   * `epoch`, the one it reported (BO-1).
   */
  onStreamRefused?(code: number, at: number, epoch?: number): void
  /** The engine stopped on its own (helper gone, session replaced). */
  onClosed(): void
}

export interface Engine {
  readonly kind: EngineKind
  /**
   * Hands the engine what a command the project applied means for it: both engines take the engine
   * bytes the page's project core encoded (the page owns the project in both modes); the helper
   * also takes the commands for its own MIDI inputs (`midi.list_inputs`, `midi.select_input`), as
   * messages for its devices, not as commands.
   */
  send(command: Command, bytes: Uint8Array): void
  /**
   * The frame of the engine's host clock at which a note was played at `timeStamp` (the page's
   * performance clock), for `live.input_note_on/off` (#499); `null` when the engine cannot tell.
   */
  frameAt?(timeStamp: number | undefined): number | null
  close(): Promise<void>
}
