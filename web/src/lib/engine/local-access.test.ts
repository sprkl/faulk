import { describe, expect, it, vi } from 'vitest'
import { absentHelperMessages, watchLocalAccess } from './local-access'

/** A permission status whose state the test changes, as the browser would on Allow or Block. */
class FakeStatus extends EventTarget {
  state: PermissionState = 'prompt'
  set(state: PermissionState) {
    this.state = state
    this.dispatchEvent(new Event('change'))
  }
}

function permissions(known: Record<string, FakeStatus>) {
  const query = vi.fn(async ({ name }: { name: string }) => {
    const status = known[name]
    if (!status) throw new TypeError(`unknown permission ${name}`)
    return status as unknown as PermissionStatus
  })
  return { query }
}

const settle = () => new Promise((r) => setTimeout(r, 0))

describe('watchLocalAccess', () => {
  it('calls back when the loopback permission turns to granted, not on block', async () => {
    const loopback = new FakeStatus()
    const onGranted = vi.fn()
    watchLocalAccess(onGranted, permissions({ 'loopback-network': loopback }))
    await settle()
    loopback.set('denied')
    expect(onGranted).not.toHaveBeenCalled()
    loopback.set('granted')
    expect(onGranted).toHaveBeenCalledTimes(1)
  })

  it('prefers loopback-network and watches only it when both names exist', async () => {
    const loopback = new FakeStatus()
    const older = new FakeStatus()
    const onGranted = vi.fn()
    const perms = permissions({ 'loopback-network': loopback, 'local-network-access': older })
    watchLocalAccess(onGranted, perms)
    await settle()
    expect(perms.query).toHaveBeenCalledTimes(1)
    older.set('granted')
    expect(onGranted).not.toHaveBeenCalled()
    loopback.set('granted')
    expect(onGranted).toHaveBeenCalledTimes(1)
  })

  it('falls back to local-network-access on browsers that only know that name', async () => {
    const older = new FakeStatus()
    const onGranted = vi.fn()
    watchLocalAccess(onGranted, permissions({ 'local-network-access': older }))
    await settle()
    older.set('granted')
    expect(onGranted).toHaveBeenCalledTimes(1)
  })

  it('does nothing in browsers without the permission or the Permissions API', async () => {
    const onGranted = vi.fn()
    const perms = permissions({})
    const stop = watchLocalAccess(onGranted, perms)
    await settle()
    expect(perms.query).toHaveBeenCalledTimes(2)
    stop()
    expect(() => watchLocalAccess(onGranted, undefined)()).not.toThrow()
    expect(onGranted).not.toHaveBeenCalled()
  })

  it('stops listening once stopped', async () => {
    const loopback = new FakeStatus()
    const onGranted = vi.fn()
    const stop = watchLocalAccess(onGranted, permissions({ 'loopback-network': loopback }))
    await settle()
    stop()
    loopback.set('granted')
    expect(onGranted).not.toHaveBeenCalled()
  })

  it('never listens when stopped before the query answered', async () => {
    const loopback = new FakeStatus()
    const add = vi.spyOn(loopback, 'addEventListener')
    const onGranted = vi.fn()
    const stop = watchLocalAccess(onGranted, permissions({ 'loopback-network': loopback }))
    stop()
    await settle()
    loopback.set('granted')
    expect(add).not.toHaveBeenCalled()
    expect(onGranted).not.toHaveBeenCalled()
  })
})

describe('watchLocalAccess state', () => {
  it('reports the state once known and at each change', async () => {
    const loopback = new FakeStatus()
    const onState = vi.fn()
    watchLocalAccess(() => {}, permissions({ 'loopback-network': loopback }), onState)
    await settle()
    expect(onState).toHaveBeenLastCalledWith('prompt')
    loopback.set('denied')
    expect(onState).toHaveBeenLastCalledWith('denied')
    loopback.set('granted')
    expect(onState).toHaveBeenLastCalledWith('granted')
    expect(onState).toHaveBeenCalledTimes(3)
  })

  it('reports nothing without the permission', async () => {
    const onState = vi.fn()
    watchLocalAccess(() => {}, permissions({}), onState)
    await settle()
    expect(onState).not.toHaveBeenCalled()
  })

  it('reports nothing once stopped', async () => {
    const loopback = new FakeStatus()
    const onState = vi.fn()
    const stop = watchLocalAccess(() => {}, permissions({ 'loopback-network': loopback }), onState)
    await settle()
    onState.mockClear()
    stop()
    loopback.set('denied')
    expect(onState).not.toHaveBeenCalled()
  })
})

describe('absentHelperMessages', () => {
  it('explains a blocked permission instead of saying no helper was found', () => {
    expect(absentHelperMessages('denied')).toEqual(['helper.localAccessBlocked'])
  })
  it('adds the pending hint to the absent message while the permission is to be asked', () => {
    expect(absentHelperMessages('prompt')).toEqual(['helper.absent', 'helper.localAccessPending'])
  })
  it('keeps the plain message when granted, unknown or unsupported', () => {
    expect(absentHelperMessages('granted')).toEqual(['helper.absent'])
    expect(absentHelperMessages(null)).toEqual(['helper.absent'])
  })
})
