import { describe, expect, it } from 'vitest'
import { dropAudioFiles, isAudioFile, type AudioDropHost } from './audio-drop'

const file = (name: string, type = '') => new File(['x'], name, { type })

function host(made: string | null = 't1') {
  const calls: { created: number; imported: [string, string[]][] } = { created: 0, imported: [] }
  const h: AudioDropHost = {
    createAudioTrack: () => {
      calls.created++
      return made
    },
    importFiles: async (track, files) => {
      calls.imported.push([track, files.map((f) => f.name)])
      return []
    },
  }
  return { h, calls }
}

describe('audio files dropped on the track list (AU-4)', () => {
  it('tells audio from other files by type, else by extension', () => {
    expect(isAudioFile(file('take', 'audio/wav'))).toBe(true)
    expect(isAudioFile(file('Loop.AIFF'))).toBe(true)
    expect(isAudioFile(file('song.mp3', 'application/octet-stream'))).toBe(true)
    expect(isAudioFile(file('notes.txt', 'text/plain'))).toBe(false)
    expect(isAudioFile(file('noextension'))).toBe(false)
  })

  it('makes one audio track and imports the audio files into it, the others left out', () => {
    const { h, calls } = host()
    expect(dropAudioFiles(h, [file('a.wav', 'audio/wav'), file('readme.txt', 'text/plain'), file('b.flac')])).toBe('browser.audioImporting')
    expect(calls.created).toBe(1)
    expect(calls.imported).toEqual([['t1', ['a.wav', 'b.flac']]])
  })

  it('makes nothing for a drop without audio, and says when the track could not be made', () => {
    const none = host()
    expect(dropAudioFiles(none.h, [file('readme.txt', 'text/plain')])).toBe('browser.audioNone')
    expect(none.calls.created).toBe(0)
    const refused = host(null)
    expect(dropAudioFiles(refused.h, [file('a.wav', 'audio/wav')])).toBe('browser.audioNoTrack')
    expect(refused.calls.imported).toEqual([])
  })
})
