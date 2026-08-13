import type { BeefApiAccountState } from '../api/tauri'

export type BeefApiAccountAction =
  | 'none'
  | 'start'
  | 'reopen'
  | 'continue'
  | 'retry'
  | 'reconnect'
  | 'manage_account'
  | 'manage_tokens'

export type BeefApiAccountMode =
  | 'loading'
  | 'signed_out'
  | 'waiting'
  | 'ready'
  | 'offline'
  | 'retry'
  | 'blocked'
  | 'cleanup'

export function hasBeefApiAccountMetadata(state: BeefApiAccountState) {
  return Boolean(
    state.email?.trim()
    && state.group?.trim()
    && state.defaultModel?.trim()
    && state.keyName?.trim(),
  )
}

export function canUseManagedBeefApiAccount(state: BeefApiAccountState) {
  if (!hasBeefApiAccountMetadata(state)) return false
  return state.phase === 'signed_in' || state.phase === 'offline'
}

export function accountPresentation(state: BeefApiAccountState): {
  mode: BeefApiAccountMode
  primaryAction: BeefApiAccountAction
  showVerification: boolean
  canCancel: boolean
} {
  if (state.phase === 'signed_out' && state.reason === 'initializing') {
    return presentation('loading', 'none')
  }
  switch (state.phase) {
    case 'signed_out':
      return presentation('signed_out', 'start')
    case 'authorizing':
    case 'polling':
      return presentation('waiting', 'reopen', true, true)
    case 'signed_in':
      return presentation('ready', 'continue')
    case 'offline':
      return presentation('offline', 'reconnect')
    case 'denied':
    case 'expired':
    case 'credential_store_failed':
      return presentation('retry', 'retry')
    case 'entitlement_missing':
      return presentation('blocked', 'manage_account')
    case 'cleanup_required':
      return presentation('cleanup', 'manage_tokens')
  }
}

export function formatAuthorizationRemaining(
  expiresAt: number | undefined,
  nowSeconds = Math.floor(Date.now() / 1_000),
): string | null {
  if (expiresAt == null) return null
  const remaining = Math.max(0, Math.floor(expiresAt - nowSeconds))
  return `${String(Math.floor(remaining / 60)).padStart(2, '0')}:${String(remaining % 60).padStart(2, '0')}`
}

function presentation(
  mode: BeefApiAccountMode,
  primaryAction: BeefApiAccountAction,
  showVerification = false,
  canCancel = false,
) {
  return { mode, primaryAction, showVerification, canCancel }
}
