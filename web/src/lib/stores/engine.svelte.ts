import type { EngineStatus } from '../engine/engine'
import type { FeedbackReport } from '../engine/feedback'
import type { MidiInput } from '../generated/protocol'
import type { MessageKey } from '../i18n/index'
import type { Refusal } from './refusal'
import { storage, TOKEN_KEY } from './storage'

export type HelperState = 'checking' | 'absent' | 'incompatible' | 'available' | 'connecting' | 'connected'

/**
 * What the engines report and how the page reaches them: status, mode, latency, meter, notes
 * sounding, MIDI inputs and the helper connection. Written only by the dispatcher, read by the
 * views.
 */
export class EngineStore {
  started = $state(false)
  starting = $state(false)
  status = $state<EngineStatus | null>(null)
  /** Which engine plays: the page's (Web) or the helper's (Boost). */
  mode = $state<'web' | 'native'>('web')
  peak = $state(0)
  voices = $state(0)
  /** The engine's last report (meters per track and master, transport, playhead, load, counters). */
  feedback = $state.raw<FeedbackReport | null>(null)
  /** Notes sounding, for the piano strip and the MIDI activity light. */
  activeNotes = $state<number[]>([])
  inputs = $state<MidiInput[]>([])
  selectedInput = $state<string | null>(null)
  midiState = $state<'unknown' | 'ok' | 'unsupported' | 'denied'>('unknown')
  helper = $state<HelperState>('checking')
  helperProtocol = $state<number | null>(null)
  /** The browser's local network permission, `null` where it does not exist or is unknown. */
  localAccess = $state<PermissionState | null>(null)
  paired = $state(Boolean(storage()?.getItem(TOKEN_KEY)))
  notice = $state<MessageKey | null>(null)
  /** The last command the core refused, shown as a temporary toast; the dispatcher clears it. */
  refusal = $state<Refusal | null>(null)
  /**
   * The Web engine stopped taking the stream (`MsgError` code, byte offset): it is behind the
   * project. The dispatcher stopped it; the shell shows the message and the Start button
   * restarts audio from a snapshot, which clears this.
   */
  streamError = $state<{ code: number; at: number } | null>(null)
  pairingMessage = $state<{ key: MessageKey; n?: number } | null>(null)
  /** Code from a `#pair=` link, used once audio has started (browsers need a click). */
  pendingPairCode = $state<string | null>(null)
}
