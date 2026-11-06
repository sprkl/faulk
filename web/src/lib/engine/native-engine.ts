import { ERROR_CODES, MAX_STREAM_BYTES, PROTOCOL_VERSION, type Capabilities, type ClientMessage, type Command, type LiveRouting } from '../generated/protocol'
import type { Engine, EngineEvents, EngineStatus, HelperUpdateState } from './engine'
import { decodeFeedback } from './feedback'
import { answerOnce } from '../agents/agent-call'
import { HELPER_URL, parseServerMessage } from './helper-client'

/** The helper answered the session with `audio.no_output`: it runs and the token is good, but it has no audio output to play on yet. */
export class NoOutputError extends Error {
  constructor() {
    super('the helper has no audio output')
    this.name = 'NoOutputError'
  }
}

/** The project's place as the helper is told it (AU-7): its id, the page's link marker, the folder's name. */
export interface HelperPlace {
  project: string
  nonce: string
  name: string
}

/** `MsgError::TooLarge`: a stream longer than the helper reads in one frame (`MAX_STREAM_BYTES`), never sent. */
export const STREAM_TOO_LARGE = -5

/**
 * The message for the helper's own MIDI inputs that a command the page ran stands for: the page runs
 * `midi.list_inputs` and `midi.select_input` in its project core like any command and, in Boost, tells
 * the helper, whose devices they are (BO-1). `null` for every other command.
 */
export function helperDeviceMessage(command: Command): ClientMessage | null {
  if (command.type === 'midi.list_inputs') return { type: 'midi.list_inputs' }
  if (command.type === 'midi.select_input') return { type: 'midi.select_input', id: command.id ?? null }
  return null
}

/**
 * The engine running natively in the helper, driven over the local WebSocket. The helper holds no
 * project (BO-1): it takes the engine bytes the page's project core encodes, one binary frame per
 * stream, the same bytes the Web engine takes, and sends back the same reports.
 */
export class NativeEngine implements Engine {
  readonly kind = 'native' as const
  private closing = false
  /** Last status the helper announced; read it after `connect` resolves, since the
   * first one arrives before the page has made this engine the current one. */
  status: EngineStatus | null = null
  /** Last update state the helper announced (#515); like `status`, the first one arrives before the engine is current. */
  update: HelperUpdateState | null = null
  /** The epoch of the helper's engine, from its hello: the page's first stream is a snapshot in a newer one. */
  epoch = 0
  /** What the helper said it can do in its hello (MIDI inputs, the project's folder). */
  capabilities: Capabilities | null = null
  /** Aborted when the session ends: the agents' calls it carried can no longer be answered (AG-4). */
  readonly calls = new AbortController()

  private constructor(
    private readonly ws: WebSocket,
    private readonly events: EngineEvents,
  ) {}

  /** Opens a session with the pairing token. Resolves once the helper said hello; rejects with `NoOutputError` when it has no audio output. */
  static connect(token: string, events: EngineEvents, timeoutMs = 3000): Promise<NativeEngine> {
    return new Promise((resolve, reject) => {
      const ws = new WebSocket(`${HELPER_URL}/session?token=${encodeURIComponent(token)}`)
      // Reports are binary frames: delivered as ArrayBuffer, not Blob (decoded synchronously).
      ws.binaryType = 'arraybuffer'
      const engine = new NativeEngine(ws, events)
      let opened = false
      const timer = setTimeout(() => {
        if (!opened) {
          ws.close()
          reject(new Error('helper session timeout'))
        }
      }, timeoutMs)
      ws.onmessage = (event) => {
        if (event.data instanceof ArrayBuffer) {
          const report = decodeFeedback(event.data)
          if (report) events.onFeedback(report)
          return
        }
        const message = parseServerMessage(event.data)
        if (!message) return
        switch (message.type) {
          case 'hello':
            if (message.protocol !== PROTOCOL_VERSION) {
              ws.close()
              if (!opened) reject(new Error(`helper protocol ${message.protocol}, page ${PROTOCOL_VERSION}`))
              return
            }
            engine.status = {
              kind: 'native',
              sampleRate: message.sample_rate,
              estimatedLatencyMs: message.estimated_latency_ms ?? null,
            }
            engine.epoch = message.epoch ?? 0
            engine.capabilities = message.capabilities ?? null
            if (!opened) {
              opened = true
              clearTimeout(timer)
              resolve(engine)
            }
            events.onStatus(engine.status)
            break
          case 'midi.inputs':
            events.onMidiInputs?.(message.inputs, message.selected ?? null)
            break
          case 'midi.message':
            events.onMidiMessage?.(message.data, message.played)
            break
          case 'stream_refused':
            events.onStreamRefused?.(message.code, message.at, message.epoch)
            break
          case 'agent.call': {
            // Answered once, on this session: the helper takes an answer only from the link it asked.
            const answer = answerOnce(message.id, (m) => engine.message(m))
            if (events.onAgentCall) events.onAgentCall(message.agent, message.command, answer, engine.calls.signal)
            else answer({ status: 'unavailable', reason: 'this page does not take agent calls' })
            break
          }
          case 'agent.left':
            events.onAgentLeft?.(message.agent)
            break
          case 'folder.state':
            events.onFolderState?.(message.project ?? null, message.state)
            break
          case 'update.status':
            engine.update = { current: message.current ?? null, checkedAt: message.checked_at ?? null, status: message.status }
            events.onHelperUpdate?.(engine.update)
            break
          case 'error':
            // Before the hello, a refusal for want of an output, or an output lost while the session opened: no
            // engine to play on. Told apart from a refused token (#525).
            if (!opened && (message.code === ERROR_CODES.no_output || message.code === ERROR_CODES.audio_stream)) {
              clearTimeout(timer)
              ws.close()
              reject(new NoOutputError())
              return
            }
            events.onError(message.code)
            break
        }
      }
      ws.onclose = () => {
        clearTimeout(timer)
        // The agents' calls of this session can no longer be answered: their questions are withdrawn (AG-4).
        engine.calls.abort()
        if (!opened) reject(new Error('helper refused the session'))
        else if (!engine.closing) events.onClosed()
      }
    })
  }

  /** The engine bytes of a command; for a command about the helper's MIDI inputs, also the message for its devices. */
  send(command: Command, bytes: Uint8Array) {
    this.push(bytes)
    const device = helperDeviceMessage(command)
    if (device) this.message(device)
  }

  /**
   * Sends a stream of the page's project core in one binary frame: a command's, or a snapshot. A
   * stream longer than the helper reads is not sent and is reported refused (`STREAM_TOO_LARGE`), as
   * the helper reports a stream its engine refuses.
   */
  push(bytes: Uint8Array) {
    if (bytes.length === 0 || this.ws.readyState !== WebSocket.OPEN) return
    if (bytes.length > MAX_STREAM_BYTES) {
      this.events.onStreamRefused?.(STREAM_TOO_LARGE, 0, this.epoch)
      return
    }
    // The core's bytes are a copy out of its own (unshared) memory (`ProjectCore.engine`).
    this.ws.send(bytes as Uint8Array<ArrayBuffer>)
  }

  /** The last table sent on this session, to send only a changed one. */
  private routed: string | null = null

  /**
   * Tells the helper where its MIDI input plays (#598, `live.routing`): the slots of the tracks the live
   * input reaches, in the epoch they belong to. Sent only when it differs from the last one sent on this
   * session; the helper then plays the input's notes itself, without waiting for the page.
   */
  route(routing: LiveRouting) {
    const key = `${routing.epoch}:${routing.tracks.join(',')}`
    if (key === this.routed || this.ws.readyState !== WebSocket.OPEN) return
    this.routed = key
    this.message({ type: 'live.routing', epoch: routing.epoch, tracks: routing.tracks })
  }

  /**
   * Tells the helper where the project lives (AU-7): in a real folder, with the link marker the page
   * wrote there (`folder.open`), or nowhere it can see (`folder.close`, `place` null). The helper never
   * gets a path from the page. Sent when it differs from the last one sent on this session.
   */
  place(place: HelperPlace | null) {
    if (this.ws.readyState !== WebSocket.OPEN) return
    const key = place ? `${place.project}:${place.nonce}:${place.name}` : ''
    if (key === this.placed) return
    this.placed = key
    this.message(place ? { type: 'folder.open', ...place } : { type: 'folder.close' })
  }

  /** The last place sent on this session (`null`: none yet, so the first one always goes). */
  private placed: string | null = null

  /** Asks the helper to show its folder dialog for the project's folder (`folder.choose`, AU-7). */
  chooseFolder(place: HelperPlace) {
    this.message({ type: 'folder.choose', ...place })
  }

  private message(message: ClientMessage) {
    if (this.ws.readyState === WebSocket.OPEN) this.ws.send(JSON.stringify(message))
  }

  /** Closes the session; the helper stops the transport and releases every note when a session ends. */
  async close() {
    this.closing = true
    // Free the MIDI input so Web mode can open it (the helper also does it on close).
    this.message({ type: 'midi.select_input', id: null })
    this.ws.close()
  }
}
