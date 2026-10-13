import { PROTOCOL_VERSION, type ClientMessage, type Command } from '../generated/protocol'
import type { Engine, EngineEvents, EngineStatus } from './engine'
import { HELPER_URL, parseServerMessage } from './helper-client'

/** The engine running natively in the helper, driven over the local WebSocket. */
export class NativeEngine implements Engine {
  readonly kind = 'native' as const
  private closing = false
  /** Last status the helper announced; read it after `connect` resolves, since the
   * first one arrives before the page has made this engine the current one. */
  status: EngineStatus | null = null

  private constructor(private readonly ws: WebSocket) {}

  /** Opens a session with the pairing token. Resolves once the helper said hello. */
  static connect(token: string, events: EngineEvents, timeoutMs = 3000): Promise<NativeEngine> {
    return new Promise((resolve, reject) => {
      const ws = new WebSocket(`${HELPER_URL}/session?token=${encodeURIComponent(token)}`)
      const engine = new NativeEngine(ws)
      let opened = false
      const timer = setTimeout(() => {
        if (!opened) {
          ws.close()
          reject(new Error('helper session timeout'))
        }
      }, timeoutMs)
      ws.onmessage = (event) => {
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
          case 'meter':
            events.onMeter(message.peak, message.voices)
            break
          case 'midi.inputs':
            events.onMidiInputs?.(message.inputs, message.selected ?? null)
            break
          case 'midi.activity':
            events.onMidiActivity?.(message.note, message.velocity)
            break
          case 'error':
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

  run(command: Command) {
    if (this.ws.readyState === WebSocket.OPEN) {
      this.ws.send(JSON.stringify({ type: 'command', command } satisfies ClientMessage))
    }
  }

  async close() {
    this.closing = true
    this.run({ type: 'note.all_off' })
    // Free the MIDI input so Web mode can open it (the helper also does it on close).
    this.run({ type: 'midi.select_input', id: null })
    this.ws.close()
  }
}
