import { describe, expect, it } from 'vitest'
import { streamErrorCode } from './project-core'

describe('streamErrorCode', () => {
  it('reads the MsgError code of an engine stream error', () => {
    expect(streamErrorCode(new Error('engine stream: error -4'))).toBe(-4)
    expect(streamErrorCode(new Error('engine stream: out of memory'))).toBe(0)
  })
})
