import type { EngineStatus } from '../engine/engine'
import type { MidiInput } from '../generated/protocol'
import type { MessageKey } from '../i18n/index'
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
  /** Notes sounding, for the piano strip and the MIDI activity light. */
  activeNotes = $state<number[]>([])
  inputs = $state<MidiInput[]>([])
  selectedInput = $state<string | null>(null)
  midiState = $state<'unknown' | 'ok' | 'unsupported' | 'denied'>('unknown')
  helper = $state<HelperState>('checking')
  helperProtocol = $state<number | null>(null)
  paired = $state(Boolean(storage()?.getItem(TOKEN_KEY)))
  notice = $state<MessageKey | null>(null)
  pairingMessage = $state<{ key: MessageKey; n?: number } | null>(null)
  /** Code from a `#pair=` link, used once audio has started (browsers need a click). */
  pendingPairCode = $state<string | null>(null)
}
