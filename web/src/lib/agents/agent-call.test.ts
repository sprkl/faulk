import { describe, expect, it, vi } from 'vitest'
import type { AgentResult, Command } from '../generated/protocol'
import { agentAuthor, answerAgentCall, NEEDS_A_CLICK, type AgentHost } from './agent-call'

function host(result: AgentResult | Error = { status: 'applied', output: { type: 'history.undo' } }) {
  const runForAgent = vi.fn<(command: Command, author: string) => Promise<AgentResult>>(async () => {
    if (result instanceof Error) throw result
    return result
  })
  return { runForAgent } satisfies AgentHost
}

describe('answerAgentCall', () => {
  it('runs the command as the agent and answers what the page did', async () => {
    const h = host({ status: 'refused', error: { code: 'not_found', entity: 'track', id: 'x' } })
    const result = await answerAgentCall(h, 'claude-code', { type: 'track.delete', track: 'x' })
    expect(h.runForAgent).toHaveBeenCalledWith({ type: 'track.delete', track: 'x' }, 'agent:claude-code')
    expect(result).toEqual({ status: 'refused', error: { code: 'not_found', entity: 'track', id: 'x' } })
  })

  it('names an agent without a name mcp', async () => {
    expect(agentAuthor('')).toBe('agent:mcp')
    expect(agentAuthor('  ')).toBe('agent:mcp')
    expect(agentAuthor('Claude Code')).toBe('agent:Claude Code')
  })

  it('answers invalid for anything that is not a command object, without running it', async () => {
    for (const bad of [null, 3, 'track.create', [], { kind: 'midi' }, { type: 7 }]) {
      const h = host()
      expect((await answerAgentCall(h, 'a', bad)).status, JSON.stringify(bad)).toBe('invalid')
      expect(h.runForAgent).not.toHaveBeenCalled()
    }
  })

  it('answers unavailable for every command that needs a click, without running it', async () => {
    expect(NEEDS_A_CLICK.size).toBe(6)
    for (const type of NEEDS_A_CLICK) {
      const h = host()
      const result = await answerAgentCall(h, 'a', { type })
      expect(result.status, type).toBe('unavailable')
      expect(h.runForAgent).not.toHaveBeenCalled()
    }
  })

  it('answers unavailable when the page fails, never throws', async () => {
    const error = vi.spyOn(console, 'error').mockImplementation(() => {})
    const result = await answerAgentCall(host(new Error('boom')), 'a', { type: 'history.undo' })
    expect(result).toEqual({ status: 'unavailable', reason: expect.stringContaining('boom') })
    error.mockRestore()
  })
})
