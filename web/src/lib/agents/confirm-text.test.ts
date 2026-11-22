import { describe, expect, it } from 'vitest'
import { translate } from '../i18n/app'
import type { AgentQuestion } from './confirm.svelte'
import { MAX_AUDIO_URL_BYTES } from '../generated/protocol'
import { agentName, MAX_AUDIO_URL_MB, questionBody, questionTitle } from './confirm-text'

const questions: AgentQuestion[] = [
  { kind: 'new' },
  { kind: 'open', target: 'Night drive' },
  { kind: 'open', target: null },
  { kind: 'delete', target: 'Old draft' },
  { kind: 'delete', target: null },
  { kind: 'keepMine', target: 'song' },
  { kind: 'loadFolder', target: 'song' },
  { kind: 'command', command: 'project.open' },
  { kind: 'importAudio', host: 'example.org', name: 'kick.wav', track: { name: 'Drums', index: 1 } },
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

  it('shows the host and the file of a link apart, the track, and the cap the page keeps (#664)', () => {
    for (const lang of ['en', 'fr'] as const) {
      const t = (key: Parameters<typeof translate>[1], vars?: Record<string, string | number>) => translate(lang, key, vars)
      const question = { kind: 'importAudio', host: 'xn--bcher-kva.example', name: 'kick.wav', track: { name: 'Drums', index: 1 } } as const
      const pending = { id: 1, agent: 'agent:a', question, deadline: 0 }
      const title = questionTitle(pending, t)
      const body = questionBody(pending, t)
      expect(title).toContain('xn--bcher-kva.example')
      // The name, chosen by the agent, is in the body, never beside the host.
      expect(title).not.toContain('kick.wav')
      expect(body).toContain('kick.wav')
      expect(body).toContain('Drums')
      expect(body).toContain(`${MAX_AUDIO_URL_MB} M`)
      expect(MAX_AUDIO_URL_MB * (1 << 20)).toBe(MAX_AUDIO_URL_BYTES)
      // A track without a name of its own is named as the track list names it.
      const unnamed = { ...pending, question: { ...question, track: { name: '  ', index: 1 } } }
      expect(questionBody(unnamed, t)).toContain(t('tracks.defaultName', { n: 2 }))
    }
  })

  it("shows the agent's own name", () => {
    expect(agentName('agent:Claude Code')).toBe('Claude Code')
    expect(agentName('agent:mcp')).toBe('mcp')
  })
})
