import { ERROR_CODES, PROTOCOL_VERSION, type ClientMessage, type Command } from '../generated/protocol'
import type { Engine, EngineEvents, EngineStatus, HelperUpdateState } from './engine'
import { decodeFeedback } from './feedback'
import { HELPER_URL, parseServerMessage } from './helper-client'
import { allNotesOff } from '../synth-commands'

/** The helper answered the session with `audio.no_output`: it runs and the token is good, but it has no audio output to play on yet. */
export class NoOutputError extends Error {
  constructor() {
    super('the helper has no audio output')
    this.name = 'NoOutputError'
  }
}

/** The engine running natively in the helper, driven over the local WebSocket. */
export class NativeEngine implements Engine {
  readonly kind = 'native' as const
  private closing = false
  /** Last status the helper announced; read it after `connect` resolves, since the
   * first one arrives before the page has made this engine the current one. */
  status: EngineStatus | null = null
  /** Last update state the helper announced (#515); like `status`, the first one arrives before the engine is current. */
  update: HelperUpdateState | null = null

  private constructor(private readonly ws: WebSocket) {}

  /** Opens a session with the pairing token. Resolves once the helper said hello; rejects with `NoOutputError` when it has no audio output. */
  static connect(token: string, events: EngineEvents, timeoutMs = 3000): Promise<NativeEngine> {
    return new Promise((resolve, reject) => {
      const ws = new WebSocket(`${HELPER_URL}/session?token=${encodeURIComponent(token)}`)
      // Reports are binary frames: delivered as ArrayBuffer, not Blob (decoded synchronously).
      ws.binaryType = 'arraybuffer'
      const engine = new NativeEngine(ws)
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
          case 'midi.activity':
            events.onMidiActivity?.(message.note, message.velocity)
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
        if (!opened) reject(new Error('helper refused the session'))
        else if (!engine.closing) events.onClosed()
      }
    })
  }

  /** The helper holds its own copy of the project until BO-1: it takes commands, not bytes. */
  send(command: Command) {
    if (this.ws.readyState === WebSocket.OPEN) {
      this.ws.send(JSON.stringify({ type: 'command', command } satisfies ClientMessage))
    }
  }

  async close() {
    this.closing = true
    this.send(allNotesOff())
    // Free the MIDI input so Web mode can open it (the helper also does it on close).
    this.send({ type: 'midi.select_input', id: null })
    this.ws.close()
  }
}
