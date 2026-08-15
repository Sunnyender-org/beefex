export type RendererDiagnostic = {
  transition: 'window_error' | 'unhandled_rejection'
  errorClass: string
  messageCode: string
}

export type RendererDiagnosticReporter = (input: RendererDiagnostic) => Promise<void>

let activeCleanup: (() => void) | null = null

function safeErrorClass(value: unknown): string {
  const candidate = value instanceof Error
    ? value.name
    : typeof value === 'object' && value && 'name' in value
      ? String(value.name)
      : 'Error'
  return /^[A-Za-z][A-Za-z0-9_]{0,63}$/.test(candidate) ? candidate : 'Error'
}

export function installDiagnosticsErrorBridge(report: RendererDiagnosticReporter): () => void {
  if (activeCleanup) return activeCleanup
  const send = (input: RendererDiagnostic) => {
    void report(input).catch(() => {})
  }
  const onError = (event: ErrorEvent) => {
    event.preventDefault()
    send({
      transition: 'window_error',
      errorClass: safeErrorClass(event.error),
      messageCode: 'renderer_window_error',
    })
  }
  const onUnhandledRejection = (event: PromiseRejectionEvent) => {
    event.preventDefault()
    send({
      transition: 'unhandled_rejection',
      errorClass: safeErrorClass(event.reason),
      messageCode: 'renderer_unhandled_rejection',
    })
  }
  window.addEventListener('error', onError)
  window.addEventListener('unhandledrejection', onUnhandledRejection)
  activeCleanup = () => {
    window.removeEventListener('error', onError)
    window.removeEventListener('unhandledrejection', onUnhandledRejection)
    activeCleanup = null
  }
  return activeCleanup
}
