import { describe, expect, it } from 'vitest'
import type { AudioMedia, Command } from '../generated/protocol'
import { dropAudioFiles, isAudioFile, placeInGrid, placeOnLane, type AudioDropHost, type LaneHost } from './audio-drop'

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

describe('audio files dropped on an audio track (AU-6)', () => {
  const media = (hash: string, frames = 48000): AudioMedia => ({ hash, sample_rate: 48000, channels: 2, frames, name: `${hash}.wav` })

  /** A page that records the commands, groups them, and makes a clip `c<n>` for each import. */
  function page(taken: [number, number][] = []) {
    const log: (Command | 'begin' | 'end')[] = []
    const clips: string[] = []
    const h: LaneHost = {
      run: (command) => {
        log.push(command)
        if (command.type === 'clip.import_audio') clips.push(`c${clips.length + 1}`)
        return true
      },
      group: (body) => {
        log.push('begin')
        try {
          return body()
        } finally {
          log.push('end')
        }
      },
      clipsOf: () => [...clips],
      free: (from, to) => !taken.some(([s, e]) => s < to && from < e),
      // One second is 1920 ticks at 120 bpm.
      ticksOf: (m) => Math.round((m.frames / 48000) * 1920),
    }
    return { h, log }
  }

  it('places each file on the lane where the last one ends, made in the arrangement where the span is free', () => {
    const { h, log } = page()
    const place = placeOnLane(h, 'track', 3840)
    expect(place(media('a'), 0)).toBe(true)
    expect(place(media('b', 24000), 1)).toBe(true)
    expect(log).toEqual([
      { type: 'clip.import_audio', track: 'track', media: 'a', name: null, at: { at: 'arrangement', start: 3840 } },
      { type: 'clip.import_audio', track: 'track', media: 'b', name: null, at: { at: 'arrangement', start: 5760 } },
    ])
  })

  it('makes the clip in the pool and places it, one history group, where the span is taken (the placement carves)', () => {
    const { h, log } = page([[4000, 5000]])
    expect(placeOnLane(h, 'track', 3840)(media('a'), 0)).toBe(true)
    expect(log).toEqual([
      'begin',
      { type: 'clip.import_audio', track: 'track', media: 'a', name: null },
      { type: 'arrangement.place', clip: 'c1', start: 3840 },
      'end',
    ])
  })

  it('puts each file into the free cell the grid gives, or the pool when none is left', () => {
    const { h, log } = page()
    const scenes = ['s1', null]
    const place = placeInGrid(h, 'track', (index) => scenes[index] ?? null)
    place(media('a'), 0)
    place(media('b'), 1)
    expect(log).toEqual([
      { type: 'clip.import_audio', track: 'track', media: 'a', name: null, at: { at: 'slot', scene: 's1' } },
      { type: 'clip.import_audio', track: 'track', media: 'b', name: null, at: null },
    ])
  })
})
