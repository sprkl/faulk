import { describe, expect, it } from 'vitest'
import { translate } from '../i18n/app'
import type { AgentQuestion } from './confirm.svelte'
import { agentName, questionBody, questionTitle } from './confirm-text'

const questions: AgentQuestion[] = [
  { kind: 'new' },
  { kind: 'open', target: 'Night drive' },
  { kind: 'open', target: null },
  { kind: 'delete', target: 'Old draft' },
  { kind: 'delete', target: null },
  { kind: 'keepMine', target: 'song' },
  { kind: 'loadFolder', target: 'song' },
  { kind: 'command', command: 'project.open' },
]

describe("an agent's question in both languages", () => {
  it('names the agent and what it would do, with no placeholder left', () => {
    for (const lang of ['en', 'fr'] as const) {
      const t = (key: Parameters<typeof translate>[1], vars?: Record<string, string | number>) => translate(lang, key, vars)
      for (const question of questions) {
        const pending = { id: 1, agent: 'agent:claude-code', question, deadline: 0 }
        const title = questionTitle(pending, t)
        const body = questionBody(pending, t)
        expect(title, `${lang} ${JSON.stringify(question)}`).toContain('claude-code')
        expect(title + body).not.toMatch(/\{\w+\}/)
        if ('target' in question && question.target && question.kind !== 'keepMine' && question.kind !== 'loadFolder') expect(title).toContain(question.target)
      }
    }
  })

  it("shows the agent's own name", () => {
    expect(agentName('agent:Claude Code')).toBe('Claude Code')
    expect(agentName('agent:mcp')).toBe('mcp')
  })
})
