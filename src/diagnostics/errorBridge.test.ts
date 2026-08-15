// @vitest-environment jsdom

import { afterEach, describe, expect, it, vi } from 'vitest'
import { installDiagnosticsErrorBridge, type RendererDiagnostic } from './errorBridge'

describe('installDiagnosticsErrorBridge', () => {
  let cleanup: (() => void) | undefined

  afterEach(() => {
    cleanup?.()
    cleanup = undefined
  })

  it('reports typed renderer failures without forwarding raw messages', async () => {
    const records: RendererDiagnostic[] = []
    cleanup = installDiagnosticsErrorBridge(async (input) => {
      records.push(input)
    })

    window.dispatchEvent(new ErrorEvent('error', {
      error: new TypeError('Bearer sk-renderer-secret /Users/tester/project'),
      message: 'Bearer sk-renderer-secret /Users/tester/project',
    }))
    const rejection = new Event('unhandledrejection', { cancelable: true }) as PromiseRejectionEvent
    Object.defineProperty(rejection, 'reason', {
      value: new RangeError('lucas@example.com'),
    })
    window.dispatchEvent(rejection)
    await Promise.resolve()

    expect(records).toEqual([
      {
        transition: 'window_error',
        errorClass: 'TypeError',
        messageCode: 'renderer_window_error',
      },
      {
        transition: 'unhandled_rejection',
        errorClass: 'RangeError',
        messageCode: 'renderer_unhandled_rejection',
      },
    ])
    expect(JSON.stringify(records)).not.toMatch(/sk-renderer-secret|lucas@example\.com|Users\/tester/)
  })

  it('swallows reporter failures instead of creating an error loop', async () => {
    const report = vi.fn().mockRejectedValue(new Error('report failed'))
    cleanup = installDiagnosticsErrorBridge(report)

    expect(() => {
      window.dispatchEvent(new ErrorEvent('error', { error: new Error('boom') }))
    }).not.toThrow()
    await Promise.resolve()

    expect(report).toHaveBeenCalledTimes(1)
  })

  it('installs only one bridge when initialization is repeated', async () => {
    const first = vi.fn().mockResolvedValue(undefined)
    const second = vi.fn().mockResolvedValue(undefined)
    cleanup = installDiagnosticsErrorBridge(first)
    const repeatedCleanup = installDiagnosticsErrorBridge(second)

    window.dispatchEvent(new ErrorEvent('error', { error: new Error('boom') }))
    await Promise.resolve()

    expect(first).toHaveBeenCalledTimes(1)
    expect(second).not.toHaveBeenCalled()
    expect(repeatedCleanup).toBe(cleanup)
  })
})
