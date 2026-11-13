import type { AgentResult, Command, FolderStatus, HelperUpdate, MidiInput, ServerMessage } from '../generated/protocol'
import type { FeedbackReport } from './feedback'

/** The helper's messages about its export (#319). */
export type HelperExportMessage = Extract<ServerMessage, { type: 'export.progress' | 'export.done' | 'export.failed' }>

/** The helper's messages about its imports and its file dialog (#778). */
export type HelperImportMessage = Extract<ServerMessage, { type: 'import.chosen' | 'import.progress' | 'import.done' | 'import.failed' }>

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
   * runs it as a Web MIDI message; `played` says the helper already played that note on its engine
   * with the page's `live.routing` (#598), so the page sends no engine bytes for it.
   */
  onMidiMessage?(data: number[], played: boolean): void
  /** Native engine only: the helper's self-update (`update.status`), at session start and on change. */
  onHelperUpdate?(update: HelperUpdateState): void
  /**
   * Native engine only: an agent's tool call the helper relays (AG-1, `agent.call`): the command it
   * sent, as JSON, and its name. `answer` sends the page's `agent.result` back on the session the call
   * came from, once. `signal` aborts when that session ends: the call can no longer be answered (AG-4).
   */
  onAgentCall?(agent: string, command: unknown, answer: (result: AgentResult) => void, signal: AbortSignal): void
  /**
   * Native engine only: where the helper's folder of the page's project stands (`folder.state`, AU-7):
   * the project it is about (`null` after `folder.close`) and its state.
   */
  onFolderState?(project: string | null, state: FolderStatus): void
  /** Native engine only: how the helper's export goes (`export.progress`, `export.done`, `export.failed`, #319). */
  onExport?(message: HelperExportMessage): void
  /** Native engine only: how the helper's imports and its file dialog go (`import.chosen`, `import.progress`, `import.done`, `import.failed`, #778). */
  onImport?(message: HelperImportMessage): void
  /** Native engine only: an agent left (`agent.left`, #641): its last MCP session ended or it stayed silent too long. */
  onAgentLeft?(agent: string): void
  onError(code: string): void
  /**
   * The engine stopped taking the message stream (`MsgError::code`: out of memory, engine busy,
   * corrupt stream; for the helper also a stream not of its engine's epoch) at byte `at` of a
   * push: it is behind the project. The Web engine is replaced by a new one that starts from a
   * snapshot (the dispatcher's restart); the helper's gets a snapshot in an epoch newer than
   * `epoch`, the one it reported (BO-1).
   */
  onStreamRefused?(code: number, at: number, epoch?: number): void
  /**
   * Web engine only: the audio output stopped pulling the engine (`true`), or pulls it again (`false`), told on each
   * change (#810): the context's clock stood still while the page expects it to run, so nothing plays and the report
   * stays frozen.
   */
  onOutputStall?(stalled: boolean): void
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
