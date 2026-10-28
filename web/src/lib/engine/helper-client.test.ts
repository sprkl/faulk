import { describe, expect, it } from 'vitest'
import { PROTOCOL_VERSION } from '../generated/protocol'
import { translate } from '../i18n/index'
import { incompatibleMessage } from './helper-client'

describe('incompatibleMessage', () => {
  it('asks an older helper for its update', () => {
    expect(incompatibleMessage(PROTOCOL_VERSION - 1)).toBe('helper.helperOlder')
    expect(incompatibleMessage(0)).toBe('helper.helperOlder')
  })

  it('asks for a newer page when the helper is newer', () => {
    expect(incompatibleMessage(PROTOCOL_VERSION + 1)).toBe('helper.helperNewer')
  })

  it('reads as decided in both languages', () => {
    expect(translate('en', 'helper.helperOlder', { v: 1 })).toBe(
      'This helper is older than the page (protocol 1). Restart it to install its update, or download it again from get.faulk.app/helper/windows.',
    )
    expect(translate('fr', 'helper.helperOlder', { v: 1 })).toBe(
      'Ce helper est plus ancien que la page (protocole 1). Relance-le pour installer sa mise à jour, ou retélécharge-le sur get.faulk.app/helper/windows.',
    )
    expect(translate('en', 'helper.helperNewer', { v: 9 })).toBe('This page is older than your helper (protocol 9). Reload the page.')
    expect(translate('fr', 'helper.helperNewer', { v: 9 })).toBe('Cette page est plus ancienne que ton helper (protocole 9). Recharge la page.')
  })
})
