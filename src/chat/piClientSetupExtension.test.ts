/* eslint-disable @typescript-eslint/no-explicit-any */
import { describe, expect, it, vi } from 'vitest'
import registerBeefexClientSetup from '../../src-tauri/resources/pi/beefex-client-setup-extension'

describe('Pi BeefAPI client setup extension', () => {
  it('registers one bounded native tool and returns the parent-owned result', async () => {
    let tool: any
    registerBeefexClientSetup({ registerTool: (definition: any) => { tool = definition } } as any)
    expect(tool.name).toBe('configure_beefapi_clients')
    expect(tool.parameters.properties.codexModel.description).toContain('not a group name')
    expect(tool.parameters.properties.codexModel.minLength).toBe(1)
    expect(tool.promptGuidelines.join(' ')).toContain('Never pass gpt-pro')
    expect(tool.promptGuidelines.join(' ')).toContain('Omit codexModel')
    const input = vi.fn().mockResolvedValue(JSON.stringify({ ok: true, configured: ['codex', 'image2', 'claude-code', 'claude-desktop', 'grok'] }))
    const result = await tool.execute('call-1', {}, undefined, undefined, { ui: { input } })
    expect(input).toHaveBeenCalledWith('__BEEFEX_MANAGED_CLIENTS_APPLY__', JSON.stringify({ codexModel: null }))
    expect(result.content[0].text).toContain('grok')
  })

  it('normalizes Pi default sentinels to the account default', async () => {
    let tool: any
    registerBeefexClientSetup({ registerTool: (definition: any) => { tool = definition } } as any)
    const input = vi.fn().mockResolvedValue(JSON.stringify({ ok: true, configured: ['codex'] }))
    await tool.execute('call-default', { codexModel: ':default' }, undefined, undefined, { ui: { input } })
    expect(input).toHaveBeenCalledWith('__BEEFEX_MANAGED_CLIENTS_APPLY__', JSON.stringify({ codexModel: null }))
  })

  it('preserves a structured parent failure', async () => {
    let tool: any
    registerBeefexClientSetup({ registerTool: (definition: any) => { tool = definition } } as any)
    const input = vi.fn().mockResolvedValue(JSON.stringify({ ok: false, error: 'managed_group_not_allowed' }))
    await expect(tool.execute('call-2', {}, undefined, undefined, { ui: { input } })).rejects.toThrow('managed_group_not_allowed')
  })
})
