import { PROTOCOL_VERSION, type ClientMessage, type ServerMessage } from '../generated/protocol'

export const HELPER_URL = 'ws://127.0.0.1:47800'

export type ProbeResult =
  | { status: 'absent' }
  | { status: 'available' }
  | { status: 'incompatible'; protocol: number }

export function parseServerMessage(data: unknown): ServerMessage | null {
  if (typeof data !== 'string') return null
  try {
    return JSON.parse(data) as ServerMessage
  } catch {
    return null
  }
}

/**
 * Asks whether a Faulk helper listens on this machine. A refused or failed connection
 * means "absent": the page cannot tell the two apart, and does not need to.
 */
export function probeHelper(timeoutMs = 1500): Promise<ProbeResult> {
  return new Promise((resolve) => {
    let settled = false
    const ws = new WebSocket(`${HELPER_URL}/probe`)
    const timer = setTimeout(() => done({ status: 'absent' }), timeoutMs)
    function done(result: ProbeResult) {
      if (settled) return
      settled = true
      clearTimeout(timer)
      ws.close()
      resolve(result)
    }
    ws.onmessage = (event) => {
      const message = parseServerMessage(event.data)
      if (message?.type !== 'probe') return done({ status: 'absent' })
      done(
        message.protocol === PROTOCOL_VERSION
          ? { status: 'available' }
          : { status: 'incompatible', protocol: message.protocol },
      )
    }
    ws.onerror = () => done({ status: 'absent' })
    ws.onclose = () => done({ status: 'absent' })
  })
}

export type PairResult =
  | { status: 'paired'; token: string }
  | { status: 'refused'; attemptsLeft: number }
  | { status: 'failed' }

/** Sends the code the helper shows; on success returns the session token. */
export function pairHelper(code: string, timeoutMs = 3000): Promise<PairResult> {
  return new Promise((resolve) => {
    let settled = false
    const ws = new WebSocket(`${HELPER_URL}/pair`)
    const timer = setTimeout(() => done({ status: 'failed' }), timeoutMs)
    function done(result: PairResult) {
      if (settled) return
      settled = true
      clearTimeout(timer)
      ws.close()
      resolve(result)
    }
    ws.onopen = () =>
      ws.send(JSON.stringify({ type: 'pair', code: code.replace(/\s+/g, '') } satisfies ClientMessage))
    ws.onmessage = (event) => {
      const message = parseServerMessage(event.data)
      if (message?.type === 'paired') done({ status: 'paired', token: message.token })
      else if (message?.type === 'pairing_refused') done({ status: 'refused', attemptsLeft: message.attempts_left })
      else done({ status: 'failed' })
    }
    ws.onerror = () => done({ status: 'failed' })
    ws.onclose = () => done({ status: 'failed' })
  })
}
