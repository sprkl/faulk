import { afterEach, describe, expect, it, vi } from 'vitest'
import { CONTROL_PUSHED_OUT } from '../generated/protocol'
import { ControlLink, type ControlEvents } from './control-link'

class FakeSocket {
  static last: FakeSocket
  static OPEN = 1
  readyState = 0
  onopen: (() => void) | null = null
  onmessage: ((event: { data: unknown }) => void) | null = null
  onclose: ((event: { code: number }) => void) | null = null
  sent: string[] = []
  closed = false
  constructor(readonly url: string) {
    FakeSocket.last = this
  }
  open() {
    this.readyState = 1
    this.onopen?.()
  }
  send(data: string) {
    this.sent.push(data)
  }
  close() {
    this.closed = true
  }
  /** The helper or the network ends the connection, with its close code (1006: no close frame). */
  drop(code = 1006) {
    this.readyState = 3
    this.onclose?.({ code })
  }
}

afterEach(() => {
  vi.unstubAllGlobals()
  vi.useRealTimers()
})

function events() {
  return { onAgentCall: vi.fn(), onAgentLeft: vi.fn(), onClosed: vi.fn() } satisfies ControlEvents
}

describe("the page's control link (#608)", () => {
  it('opens the control route with the token and resolves once the socket is open', async () => {
    vi.stubGlobal('WebSocket', FakeSocket)
    const opening = ControlLink.open('a b', events())
    const socket = FakeSocket.last
    expect(socket.url).toMatch(/\/control\?token=a%20b$/)
    socket.open()
    const link = await opening
    expect(link.token).toBe('a b')
  })

  it('rejects a link the helper refuses, or that takes too long', async () => {
    vi.stubGlobal('WebSocket', FakeSocket)
    const refused = ControlLink.open('t', events())
    FakeSocket.last.drop()
    await expect(refused).rejects.toThrow()

    vi.useFakeTimers()
    const slow = ControlLink.open('t', events(), 1000)
    const socket = FakeSocket.last
    vi.advanceTimersByTime(1000)
    await expect(slow).rejects.toThrow(/timeout/)
    expect(socket.closed).toBe(true)
  })

  it('hands each agent call over and sends its answer once, on this link; ignores anything else', async () => {
    vi.stubGlobal('WebSocket', FakeSocket)
    const told = events()
    const opening = ControlLink.open('t', told)
    const socket = FakeSocket.last
    socket.open()
    await opening
    socket.onmessage!({ data: 'not json' })
    socket.onmessage!({ data: JSON.stringify({ type: 'midi.inputs', inputs: [], selected: null }) })
    expect(told.onAgentCall).not.toHaveBeenCalled()
    socket.onmessage!({ data: JSON.stringify({ type: 'agent.call', id: 7, agent: 'claude-code', command: { type: 'history.undo' } }) })
    expect(told.onAgentCall).toHaveBeenCalledTimes(1)
    const [agent, command, answer] = told.onAgentCall.mock.calls[0]
    expect(agent).toBe('claude-code')
    expect(command).toEqual({ type: 'history.undo' })
    answer({ status: 'applied', output: null })
    answer({ status: 'invalid', message: 'twice' })
    expect(socket.sent.map((s) => JSON.parse(s))).toEqual([{ type: 'agent.result', id: 7, result: { status: 'applied', output: null } }])
  })

  it('tells that an agent left (#641)', async () => {
    vi.stubGlobal('WebSocket', FakeSocket)
    const told = events()
    const opening = ControlLink.open('t', told)
    const socket = FakeSocket.last
    socket.open()
    await opening
    socket.onmessage!({ data: JSON.stringify({ type: 'agent.left', agent: 'claude-code' }) })
    expect(told.onAgentLeft).toHaveBeenCalledExactlyOnceWith('claude-code')
    expect(told.onAgentCall).not.toHaveBeenCalled()
  })

  it('says it came to the front, tells a loss, and not its own close', async () => {
    vi.stubGlobal('WebSocket', FakeSocket)
    const told = events()
    const opening = ControlLink.open('t', told)
    const socket = FakeSocket.last
    socket.open()
    const link = await opening
    link.front()
    expect(JSON.parse(socket.sent[0])).toEqual({ type: 'control.front' })
    socket.drop()
    expect(told.onClosed).toHaveBeenCalledExactlyOnceWith(false)
    // An answer after the loss goes nowhere.
    socket.onmessage!({ data: JSON.stringify({ type: 'agent.call', id: 8, agent: 'a', command: {} }) })
    told.onAgentCall.mock.calls[0][2]({ status: 'applied', output: null })
    expect(socket.sent).toHaveLength(1)

    const quiet = events()
    const second = ControlLink.open('t', quiet)
    const other = FakeSocket.last
    other.open()
    ;(await second).close()
    expect(other.closed).toBe(true)
    other.drop()
    expect(quiet.onClosed).not.toHaveBeenCalled()
  })

  it("aborts its calls' signal when it closes, whoever closes it: their questions are withdrawn (AG-4)", async () => {
    vi.stubGlobal('WebSocket', FakeSocket)
    const told = events()
    const opening = ControlLink.open('t', told)
    const socket = FakeSocket.last
    socket.open()
    await opening
    socket.onmessage!({ data: JSON.stringify({ type: 'agent.call', id: 7, agent: 'a', command: { type: 'project.new' } }) })
    const signal = told.onAgentCall.mock.calls[0][3] as AbortSignal
    expect(signal.aborted).toBe(false)
    socket.drop()
    expect(signal.aborted).toBe(true)

    const mine = events()
    const second = ControlLink.open('t', mine)
    const other = FakeSocket.last
    other.open()
    const link = await second
    other.onmessage!({ data: JSON.stringify({ type: 'agent.call', id: 8, agent: 'a', command: { type: 'project.new' } }) })
    const own = mine.onAgentCall.mock.calls[0][3] as AbortSignal
    link.close()
    other.drop()
    expect(own.aborted).toBe(true)
  })

  it('says when the helper pushed the link out for newer ones', async () => {
    vi.stubGlobal('WebSocket', FakeSocket)
    const told = events()
    const opening = ControlLink.open('t', told)
    const socket = FakeSocket.last
    socket.open()
    await opening
    socket.drop(CONTROL_PUSHED_OUT)
    expect(told.onClosed).toHaveBeenCalledExactlyOnceWith(true)
  })
})
