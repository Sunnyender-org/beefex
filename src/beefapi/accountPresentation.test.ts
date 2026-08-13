import { describe, expect, it } from 'vitest'
import type { BeefApiAccountPhase, BeefApiAccountState } from '../api/tauri'
import {
  accountPresentation,
  canUseManagedBeefApiAccount,
  formatAuthorizationRemaining,
  hasBeefApiAccountMetadata,
} from './accountPresentation'

const account = (phase: BeefApiAccountPhase, reason?: string): BeefApiAccountState => ({
  phase,
  reason,
})

describe('accountPresentation', () => {
  it('keeps startup reconciliation distinct from the final signed-out state', () => {
    expect(accountPresentation(account('signed_out', 'initializing')).mode).toBe('loading')
    expect(accountPresentation(account('signed_out')).primaryAction).toBe('start')
  })

  it('maps every authorization phase to one explicit user action', () => {
    const expected: Record<BeefApiAccountPhase, string> = {
      signed_out: 'start',
      authorizing: 'reopen',
      polling: 'reopen',
      signed_in: 'continue',
      denied: 'retry',
      expired: 'retry',
      offline: 'reconnect',
      entitlement_missing: 'manage_account',
      credential_store_failed: 'retry',
      cleanup_required: 'manage_tokens',
    }

    for (const [phase, primaryAction] of Object.entries(expected)) {
      expect(accountPresentation(account(phase as BeefApiAccountPhase)).primaryAction).toBe(primaryAction)
    }
  })

  it('only exposes verification and cancellation while an authorization is active', () => {
    expect(accountPresentation(account('polling'))).toMatchObject({
      showVerification: true,
      canCancel: true,
    })
    expect(accountPresentation(account('signed_in'))).toMatchObject({
      showVerification: false,
      canCancel: false,
    })
  })

  it('does not confuse a pre-login network failure with a cached signed-in account', () => {
    const preLoginOffline = account('offline', 'network_unavailable')
    const cachedAccountOffline: BeefApiAccountState = {
      ...preLoginOffline,
      email: 'user@example.com',
      group: 'gpt-pro',
      defaultModel: 'gpt-5.6-sol',
      keyName: 'Beefex desktop',
    }

    expect(hasBeefApiAccountMetadata(preLoginOffline)).toBe(false)
    expect(canUseManagedBeefApiAccount(preLoginOffline)).toBe(false)
    expect(hasBeefApiAccountMetadata(cachedAccountOffline)).toBe(true)
    expect(canUseManagedBeefApiAccount(cachedAccountOffline)).toBe(true)
  })
})

describe('formatAuthorizationRemaining', () => {
  it('formats epoch seconds as a stable minute-second countdown', () => {
    expect(formatAuthorizationRemaining(1_600, 1_000)).toBe('10:00')
    expect(formatAuthorizationRemaining(1_061, 1_000)).toBe('01:01')
    expect(formatAuthorizationRemaining(999, 1_000)).toBe('00:00')
  })

  it('returns null when the server supplied no expiry', () => {
    expect(formatAuthorizationRemaining(undefined, 1_000)).toBeNull()
  })
})
