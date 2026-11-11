import { describe, expect, it } from 'vitest'
import type { FolderRefusal, FolderStatus } from '../generated/protocol'
import { messages } from '../i18n/app'
import { boostFolderNotice } from './boost-folder'

describe('what the Clips panel says of audio clips in Boost (AU-7)', () => {
  it('says nothing once the helper reads the folder, nor before it answers', () => {
    expect(boostFolderNotice(false, { state: { status: 'open', name: 'Song' } })).toBeNull()
    expect(boostFolderNotice(false, { state: { status: 'closed' } })).toBeNull()
    expect(boostFolderNotice(false, null)).toBeNull()
  })

  it('a project kept in the browser asks for a folder, not for the helper', () => {
    expect(boostFolderNotice(true, { state: { status: 'open', name: 'x' } })).toEqual({ key: 'clips.boostBrowser', ask: false })
  })

  it('a folder that refused the marker says so and offers to try again', () => {
    expect(boostFolderNotice(false, null, true)).toEqual({ key: 'clips.boostNoMarker', ask: true })
    expect(messages.en['clips.boostNoMarker']).toBeTruthy()
    expect(messages.fr['clips.boostNoMarker']).toBeTruthy()
  })

  it('offers to show the folder whenever the user can do something about it', () => {
    const cases: [FolderStatus, string, boolean][] = [
      [{ status: 'unknown' }, 'clips.boostShowFolder', true],
      [{ status: 'ambiguous', count: 2 }, 'clips.boostAmbiguous', true],
      [{ status: 'choosing' }, 'clips.boostChoosing', false],
    ]
    const reasons: [FolderRefusal, string, boolean][] = [
      ['cancelled', 'clips.boostShowFolder', true],
      ['not_this_folder', 'clips.boostNotThisFolder', true],
      ['other_project', 'clips.boostOtherProject', true],
      ['no_project', 'clips.boostNoProject', true],
      ['unreadable', 'clips.boostUnreadable', true],
      ['unavailable', 'clips.boostNoDialog', false],
    ]
    for (const [reason, key, ask] of reasons) cases.push([{ status: 'refused', reason }, key, ask])
    for (const [state, key, ask] of cases) {
      const notice = boostFolderNotice(false, { state })
      expect(notice, JSON.stringify(state)).toMatchObject({ key, ask })
      // Every text exists in both languages.
      expect(messages.en[notice!.key], key).toBeTruthy()
      expect(messages.fr[notice!.key], key).toBeTruthy()
    }
    expect(boostFolderNotice(false, { state: { status: 'ambiguous', count: 3 } })?.vars).toEqual({ count: 3 })
  })
})
