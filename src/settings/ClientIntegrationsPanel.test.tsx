// @vitest-environment jsdom

import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { ClientIntegrationsPanel } from './ClientIntegrationsPanel'

const clients = ['codex', 'image2', 'claude-code', 'claude-desktop', 'grok'].map((id) => ({
  id,
  detected: true,
  configured: false,
  launchCommand: id === 'claude-code' ? 'claude' : id,
}))

const mocks = vi.hoisted(() => ({
  inspect: vi.fn(),
  apply: vi.fn(),
  verify: vi.fn(),
  rollback: vi.fn(),
  account: { phase: 'signed_in', defaultModel: 'gpt-5.6-terra', allowedModels: ['gpt-5.6-terra', 'gpt-5.6-sol'] },
}))

vi.mock('../api/tauri', () => ({ api: {
  managedClientsInspect: mocks.inspect,
  managedClientsApply: mocks.apply,
  managedClientsVerify: mocks.verify,
  managedClientsRollback: mocks.rollback,
} }))

vi.mock('../beefapi/useBeefApiAccount', () => ({ useBeefApiAccount: () => ({ state: mocks.account }) }))

describe('ClientIntegrationsPanel', () => {
  beforeEach(() => {
    for (const mock of [mocks.inspect, mocks.apply, mocks.verify, mocks.rollback]) mock.mockReset()
    mocks.inspect.mockResolvedValue({ state: 'ready', clients })
  })

  it('configures all fixed clients from the existing login without asking for another credential', async () => {
    mocks.apply.mockResolvedValue({ operation: 'apply', status: { state: 'configured', clients: clients.map((client) => ({ ...client, configured: true })) }, changedPaths: [], checks: [] })
    const user = userEvent.setup()
    render(<ClientIntegrationsPanel lang="zh" />)

    expect(await screen.findByText('Codex')).toBeInTheDocument()
    expect(screen.getByText('BeefAPI Image2')).toBeInTheDocument()
    expect(screen.getByText('Claude Code')).toBeInTheDocument()
    expect(screen.getByText('Claude Desktop')).toBeInTheDocument()
    expect(screen.getByText('Grok')).toBeInTheDocument()
    expect(screen.queryByRole('textbox')).not.toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: /全部配置好/ }))

    await waitFor(() => expect(mocks.apply).toHaveBeenCalledWith('gpt-5.6-terra'))
    expect(await screen.findByText(/已用当前 Beefex 登录完成/)).toBeInTheDocument()
  })

  it('keeps configuration disabled until BeefAPI is signed in', async () => {
    mocks.account.phase = 'signed_out'
    render(<ClientIntegrationsPanel lang="zh" />)
    expect(await screen.findByRole('button', { name: /全部配置好/ })).toBeDisabled()
    expect(screen.getByText('请先登录 BeefAPI。')).toBeInTheDocument()
    mocks.account.phase = 'signed_in'
  })
})
