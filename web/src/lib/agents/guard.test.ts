import { describe, expect, it } from 'vitest'
import { AGENT_CONFIRM_MS, DESTRUCTIVE_COMMANDS, type Command } from '../generated/protocol'
import { agentQuestion, confirmDeadline, notConfirmed, STORAGE_GUARDED, type GuardView } from './guard'

const view: GuardView = {
  recent: [{ id: 'r1', name: 'Folder song' }],
  browserProjects: [{ dir: 'd1', name: 'Draft' }],
  reopenTarget: { name: 'Last time' },
  folderName: 'song',
  inConflict: true,
}

const ask = (command: object, v: GuardView | null = view) => agentQuestion(command as Command, v)

describe('agentQuestion', () => {
  it('asks before every destructive command of the catalog', () => {
    expect(DESTRUCTIVE_COMMANDS.length).toBeGreaterThan(0)
    for (const type of DESTRUCTIVE_COMMANDS) expect(ask({ type }), type).not.toBeNull()
    expect(ask({ type: 'project.open', folder: 'x' })).toEqual({ kind: 'command', command: 'project.open' })
  })

  it('names the project each storage action is about', () => {
    expect(ask({ type: 'project.new' })).toEqual({ kind: 'new' })
    expect(ask({ type: 'project.open_recent', id: 'r1' })).toEqual({ kind: 'open', target: 'Folder song' })
    expect(ask({ type: 'project.open_browser_project', dir: 'd1' })).toEqual({ kind: 'open', target: 'Draft' })
    expect(ask({ type: 'project.reopen_last' })).toEqual({ kind: 'open', target: 'Last time' })
    expect(ask({ type: 'project.open_url', url: 'https://example.org/a.faulkpack' })).toEqual({ kind: 'open', target: 'example.org' })
    expect(ask({ type: 'project.delete_browser_project', dir: 'd1' })).toEqual({ kind: 'delete', target: 'Draft' })
    expect(ask({ type: 'project.keep_mine' })).toEqual({ kind: 'keepMine', target: 'song' })
    expect(ask({ type: 'project.load_folder_version' })).toEqual({ kind: 'loadFolder', target: 'song' })
    // Unknown names, a link that does not parse, no storage at all: still asked, without a name.
    expect(ask({ type: 'project.open_recent', id: 'nope' })).toEqual({ kind: 'open', target: null })
    expect(ask({ type: 'project.open_url', url: 'not a link' })).toEqual({ kind: 'open', target: null })
    for (const type of Object.keys(STORAGE_GUARDED)) expect(ask({ type }, null), type).not.toBeNull()
  })

  it('asks nothing for a way out of a conflict when there is none, nor for what can be undone', () => {
    const calm = { ...view, inConflict: false }
    expect(ask({ type: 'project.keep_mine' }, calm)).toBeNull()
    expect(ask({ type: 'project.load_folder_version' }, calm)).toBeNull()
    for (const type of ['track.delete', 'clip.delete', 'arrangement.delete', 'history.undo', 'project.save', 'project.list_stored']) {
      expect(ask({ type }), type).toBeNull()
    }
  })
})

describe('what the agent is told', () => {
  it('gives each question AGENT_CONFIRM_MS from the moment its call came', () => {
    expect(confirmDeadline(1000)).toBe(1000 + AGENT_CONFIRM_MS)
  })

  it('answers a typed refusal for a refusal and a timeout, unavailable for a withdrawn call', () => {
    expect(notConfirmed('declined')).toMatchObject({ status: 'refused', error: { code: 'confirmation.declined' } })
    expect(notConfirmed('timeout')).toMatchObject({ status: 'refused', error: { code: 'confirmation.timeout', message: expect.stringContaining('45 s') } })
    expect(notConfirmed('withdrawn').status).toBe('unavailable')
  })
})
