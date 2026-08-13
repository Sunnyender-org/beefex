import { describe, expect, it, vi } from 'vitest'
import beefexPolicyExtension from '../../src-tauri/resources/pi/beefex-policy-extension'

type ToolEvent = { toolCallId: string; toolName: string; input: Record<string, unknown> }
type ToolContext = { hasUI: boolean; ui: { confirm: (title: string, message: string) => Promise<boolean> } }
type Handler = (event: ToolEvent, context: ToolContext) => Promise<{ block: true; reason: string } | undefined>

function captureHandler(): Handler {
  let handler: Handler | undefined
  beefexPolicyExtension({
    on: (_eventName, next) => {
      handler = next
    },
  })
  if (!handler) throw new Error('tool_call handler was not registered')
  return handler
}

describe('Pi project policy extension', () => {
  it('rejects resolved paths outside the project root before approval', async () => {
    const confirm = vi.fn(async () => true)
    const result = await captureHandler()(
      { toolCallId: 'tool-1', toolName: 'write', input: { path: '/tmp/outside.txt' } },
      { hasUI: true, ui: { confirm } },
    )

    expect(result).toEqual({ block: true, reason: 'path_outside_project_root' })
    expect(confirm).not.toHaveBeenCalled()
  })

  it('blocks an inside-project write when the exact confirmation is denied', async () => {
    const confirm = vi.fn(async () => false)
    const result = await captureHandler()(
      { toolCallId: 'tool-2', toolName: 'write', input: { path: 'output/pi-policy-test.txt' } },
      { hasUI: true, ui: { confirm } },
    )

    expect(result).toEqual({ block: true, reason: 'tool_denied_by_user' })
    expect(confirm).toHaveBeenCalledWith(expect.stringContaining('tool-2'), expect.stringContaining('output/pi-policy-test.txt'))
  })

  it('allows a simple scoped validation command only after confirmation', async () => {
    const confirm = vi.fn(async () => true)
    const result = await captureHandler()(
      { toolCallId: 'tool-3', toolName: 'bash', input: { command: 'npm test -- --run src/chat/piPolicyExtension.test.ts' } },
      { hasUI: true, ui: { confirm } },
    )

    expect(result).toBeUndefined()
    expect(confirm).toHaveBeenCalledOnce()
  })

  it('rejects shell syntax whose project scope cannot be proved', async () => {
    const confirm = vi.fn(async () => true)
    const result = await captureHandler()(
      { toolCallId: 'tool-4', toolName: 'bash', input: { command: 'cd .. && touch escaped.txt' } },
      { hasUI: true, ui: { confirm } },
    )

    expect(result).toEqual({ block: true, reason: 'shell_scope_not_provable' })
    expect(confirm).not.toHaveBeenCalled()
  })
})
