import type { Command, MidiInput } from '../generated/protocol'

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
  onMeter(peak: number, voices: number): void
  /** Native engine only: its MIDI inputs (the page owns MIDI in Web mode). */
  onMidiInputs?(inputs: MidiInput[], selected: string | null): void
  onMidiActivity?(note: number, velocity: number): void
  onError(code: string): void
  /**
   * The engine stopped taking the message stream (`MsgError::code`: out of memory, engine busy,
   * corrupt stream) at byte `at` of a push: it is behind the project until a resync (PR-5).
   */
  onStreamRefused?(code: number, at: number): void
  /** The engine stopped on its own (helper gone, session replaced). */
  onClosed(): void
}

export interface Engine {
  readonly kind: EngineKind
  /** Runs a command. Commands the engine does not handle are ignored. */
  run(command: Command): void
  close(): Promise<void>
}
