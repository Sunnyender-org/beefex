// @vitest-environment jsdom

import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { ClientIntegrationsPanel } from './ClientIntegrationsPanel'

const mocks = vi.hoisted(() => ({
  inspect: vi.fn(),
  preview: vi.fn(),
  apply: vi.fn(),
  verify: vi.fn(),
  rollback: vi.fn(),
  account: {
    phase: 'signed_in',
    defaultModel: 'gpt-5.6-terra',
    allowedModels: ['gpt-5.6-terra', 'gpt-5.6-sol'],
  },
}))

vi.mock('../api/tauri', () => ({
  api: {
    codexPluginInspect: mocks.inspect,
    codexPluginPreview: mocks.preview,
    codexPluginApply: mocks.apply,
    codexPluginVerify: mocks.verify,
    codexPluginRollback: mocks.rollback,
  },
}))

vi.mock('../beefapi/useBeefApiAccount', () => ({
  useBeefApiAccount: () => ({ state: mocks.account }),
}))

const ready = {
  state: 'ready',
  codexVersion: '0.146.0',
  supported: true,
  codexHome: '/tmp/codex',
  profilePath: '/tmp/codex/beefapi.config.toml',
  credentialPresent: false,
  launchCommand: 'codex --profile beefapi',
}

describe('ClientIntegrationsPanel', () => {
  beforeEach(() => {
    for (const mock of [mocks.inspect, mocks.preview, mocks.apply, mocks.verify, mocks.rollback]) {
      mock.mockReset()
    }
    mocks.inspect.mockResolvedValue(ready)
  })

  it('uses a server-allowed model and clears the API token after apply', async () => {
    mocks.apply.mockResolvedValue({
      operation: 'apply',
      status: { ...ready, state: 'configured', credentialPresent: true, configuredModel: 'gpt-5.6-terra' },
      changedPaths: ['/tmp/codex/beefapi.config.toml'],
      configValid: true,
    })
    const user = userEvent.setup()
    render(<ClientIntegrationsPanel lang="zh" />)

    expect(await screen.findByText('0.146.0 · /tmp/codex/beefapi.config.toml')).toBeInTheDocument()
    const input = screen.getByPlaceholderText('不会写入配置或回执')
    await user.type(input, 'separate-token-value')
    await user.click(screen.getByRole('button', { name: /配置 Codex/ }))

    await waitFor(() => expect(mocks.apply).toHaveBeenCalledWith('gpt-5.6-terra', 'separate-token-value'))
    expect(input).toHaveValue('')
    expect(await screen.findByText('Codex BeefAPI 配置已写入并读回校验。')).toBeInTheDocument()
  })

  it('surfaces an unowned profile conflict and keeps write actions disabled', async () => {
    mocks.inspect.mockResolvedValue({
      ...ready,
      state: 'conflict',
      reason: 'codex_plugin_profile_conflict',
    })
    const user = userEvent.setup()
    render(<ClientIntegrationsPanel lang="zh" />)

    expect(await screen.findByText('codex_plugin_profile_conflict')).toBeInTheDocument()
    await user.type(screen.getByPlaceholderText('不会写入配置或回执'), 'separate-token-value')
    expect(screen.getByRole('button', { name: /配置 Codex/ })).toBeDisabled()
    expect(mocks.apply).not.toHaveBeenCalled()
  })

  it('shows an exact secret-free preview before any write', async () => {
    mocks.preview.mockResolvedValue({
      status: ready,
      model: 'gpt-5.6-terra',
      configPreview: '# beefex-managed-codex-profile-v1\nwire_api = "responses"\n',
      changes: [{ path: '/tmp/codex/beefapi.config.toml', action: 'create', description: 'profile' }],
      credentialContract: 'Separate token; never exported.',
    })
    const user = userEvent.setup()
    render(<ClientIntegrationsPanel lang="zh" />)

    await screen.findByText('0.146.0 · /tmp/codex/beefapi.config.toml')
    await user.click(screen.getByRole('button', { name: '预览变更' }))

    expect(await screen.findByTestId('codex-profile-preview')).toHaveTextContent('wire_api = "responses"')
    expect(mocks.apply).not.toHaveBeenCalled()
  })
})
