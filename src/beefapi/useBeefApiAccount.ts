import { useCallback, useEffect, useState } from 'react'
import { api, isTauriRuntime, type BeefApiAccountState } from '../api/tauri'

type AccountOperation = 'start' | 'cancel' | 'reopen' | 'reconnect' | 'logout' | null

const INITIAL_ACCOUNT: BeefApiAccountState = {
  phase: 'signed_out',
  reason: 'initializing',
}

export function useBeefApiAccount() {
  const [state, setState] = useState<BeefApiAccountState>(INITIAL_ACCOUNT)
  const [operation, setOperation] = useState<AccountOperation>(null)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    if (!isTauriRuntime()) {
      setState({ phase: 'signed_out' })
      return
    }
    let disposed = false
    let unlisten: (() => void) | undefined
    void api.onBeefapiAccountState((next) => {
      if (!disposed) setState(next)
    }).then(async (dispose) => {
      if (disposed) {
        dispose()
        return
      }
      unlisten = dispose
      try {
        const current = await api.beefapiAccountState()
        if (!disposed) setState(current)
      } catch (caught) {
        if (!disposed) setError(errorMessage(caught))
      }
    }).catch((caught) => {
      if (!disposed) setError(errorMessage(caught))
    })
    return () => {
      disposed = true
      unlisten?.()
    }
  }, [])

  const run = useCallback(async (
    nextOperation: Exclude<AccountOperation, null>,
    action: () => Promise<BeefApiAccountState>,
  ) => {
    setOperation(nextOperation)
    setError(null)
    try {
      const next = await action()
      setState(next)
      return next
    } catch (caught) {
      setError(errorMessage(caught))
      return null
    } finally {
      setOperation(null)
    }
  }, [])

  const startAuthorization = useCallback(
    () => run('start', api.beefapiAuthStart),
    [run],
  )
  const cancelAuthorization = useCallback(
    () => run('cancel', async () => api.beefapiAuthCancel()),
    [run],
  )
  const reconnect = useCallback(
    () => run('reconnect', api.beefapiAccountReconnect),
    [run],
  )
  const logout = useCallback(
    () => run('logout', api.beefapiLogout),
    [run],
  )
  const reopenBrowser = useCallback(async () => {
    setOperation('reopen')
    setError(null)
    try {
      await api.beefapiAuthReopenBrowser()
    } catch (caught) {
      setError(errorMessage(caught))
    } finally {
      setOperation(null)
    }
  }, [])

  return {
    state,
    operation,
    error,
    clearError: () => setError(null),
    startAuthorization,
    cancelAuthorization,
    reopenBrowser,
    reconnect,
    logout,
  }
}

function errorMessage(error: unknown) {
  if (error instanceof Error) return error.message
  return String(error)
}
