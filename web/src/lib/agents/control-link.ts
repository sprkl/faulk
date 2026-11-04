import { CONTROL_PUSHED_OUT, type AgentResult, type ClientMessage } from '../generated/protocol'
import { HELPER_URL, parseServerMessage } from '../engine/helper-client'
import { answerOnce } from './agent-call'

/** What a control link tells the page. */
export interface ControlEvents {
  /**
   * An agent's call (`agent.call`): `answer` sends the page's `agent.result` on this link, once. `signal` aborts when
   * the link closes: the call can no longer be answered (AG-4).
   */
  onAgentCall(agent: string, command: unknown, answer: (result: AgentResult) => void, signal: AbortSignal): void
  /** An agent left (`agent.left`, #641): its last MCP session ended or it stayed silent too long. */
  onAgentLeft(agent: string): void
  /**
   * The link closed after it opened: the helper went away or restarted, or (`pushedOut`) it closed this link because
   * newer ones went past its limit (`CONTROL_PUSHED_OUT`).
   */
  onClosed(pushedOut: boolean): void
}

/** The link did not open in time: a busy helper, not a refusal of its token (which closes it at once). */
export class ControlTimeout extends Error {
  constructor() {
    super('helper control link timeout')
    this.name = 'ControlTimeout'
  }
}

/**
 * The page's control link to the helper (#608): the agents' relay only (`agent.call` and `agent.left` in, `agent.result` out). A paired
 * page holds it in Web mode as in Boost, whether or not the helper has an audio output; it never takes the audio role
 * (that is the Boost session's, `NativeEngine`). The helper sends an agent's call to the Boost session if there is
 * one, else to the front control link: the newest, or the last that said `control.front`.
 */
export class ControlLink {
  private closing = false
  /** Aborted when the link closes, whoever closes it: its agents' calls can no longer be answered. */
  private readonly calls = new AbortController()

  private constructor(
    private readonly ws: WebSocket,
    /** The pairing token the link was opened with. */
    readonly token: string,
  ) {}

  /**
   * Opens the link with the pairing token. Resolves once the socket is open: the helper accepted the token in the
   * handshake (it says nothing more). Rejects when it closes, fails or takes longer than `timeoutMs` first.
   */
  static open(token: string, events: ControlEvents, timeoutMs = 3000): Promise<ControlLink> {
    return new Promise((resolve, reject) => {
      const ws = new WebSocket(`${HELPER_URL}/control?token=${encodeURIComponent(token)}`)
      const link = new ControlLink(ws, token)
      let opened = false
      const timer = setTimeout(() => {
        if (opened) return
        ws.close()
        reject(new ControlTimeout())
      }, timeoutMs)
      ws.onopen = () => {
        opened = true
        clearTimeout(timer)
        resolve(link)
      }
      ws.onmessage = (event) => {
        const message = parseServerMessage(event.data)
        if (message?.type === 'agent.left') return events.onAgentLeft(message.agent)
        if (message?.type !== 'agent.call') return
        events.onAgentCall(
          message.agent,
          message.command,
          answerOnce(message.id, (m) => link.send(m)),
          link.calls.signal,
        )
      }
      ws.onclose = (event: { code?: number }) => {
        clearTimeout(timer)
        link.calls.abort()
        if (!opened) reject(new Error('helper refused the control link'))
        else if (!link.closing) events.onClosed(event.code === CONTROL_PUSHED_OUT)
      }
    })
  }

  /** The page came to the front: agents' calls come here while no Boost session is open (`control.front`). */
  front() {
    this.send({ type: 'control.front' })
  }

  private send(message: ClientMessage) {
    if (this.ws.readyState === WebSocket.OPEN) this.ws.send(JSON.stringify(message))
  }

  /** Closes the link; its waiting calls end on the helper's side (`page.gone`). */
  close() {
    this.closing = true
    this.ws.close()
  }
}
