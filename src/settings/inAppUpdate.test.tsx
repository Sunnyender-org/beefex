import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { useState } from 'react'
import { describe, expect, it, vi } from 'vitest'
import { InAppUpdateActions } from './InAppUpdateActions'
import {
  downloadAndInstallUpdate,
  inAppUpdateFailureText,
  type InAppDownloadState,
} from './inAppUpdate'

const failureLabels = {
  downloadFailed: '下载失败',
  installFailed: '安装失败',
}

function OneClickUpdateProbe({
  downloadUpdate,
  installUpdate,
}: {
  downloadUpdate: (version: string, sha256?: string) => Promise<string>
  installUpdate: (path: string, version?: string) => Promise<void>
}) {
  const [state, setState] = useState<InAppDownloadState>('idle')
  const [error, setError] = useState('')
  return (
    <InAppUpdateActions
      state={state}
      percent={0}
      error={error}
      downloadAndInstallLabel="下载并安装"
      downloadingLabel="下载中"
      retryLabel="重试"
      openDownloadPageLabel="打开下载页"
      laterLabel="稍后"
      onDownloadAndInstall={() => {
        void (async () => {
          setState('downloading')
          setError('')
          const result = await downloadAndInstallUpdate({
            version: '0.1.0-alpha.7',
            sha256: 'abc123',
            downloadUpdate,
            installUpdate,
          })
          if (!result.ok) {
            setError(inAppUpdateFailureText(result.phase, result.error, failureLabels))
            setState('failed')
          }
        })()
      }}
      onOpenReleasePage={() => {}}
      onLater={() => {}}
    />
  )
}

describe('one-click in-app download and install', () => {
  it('calls downloadUpdate then installUpdate from a single click', async () => {
    const user = userEvent.setup()
    const downloadUpdate = vi.fn().mockResolvedValue('/tmp/beefex-desktop-win-x64.exe')
    const installUpdate = vi.fn().mockResolvedValue(undefined)
    render(
      <OneClickUpdateProbe
        downloadUpdate={downloadUpdate}
        installUpdate={installUpdate}
      />,
    )

    expect(screen.queryByText('安装并重启')).not.toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: /下载并安装/ }))

    await waitFor(() => {
      expect(downloadUpdate).toHaveBeenCalledWith('0.1.0-alpha.7', 'abc123')
      expect(installUpdate).toHaveBeenCalledWith(
        '/tmp/beefex-desktop-win-x64.exe',
        '0.1.0-alpha.7',
      )
    })
    expect(downloadUpdate.mock.invocationCallOrder[0]).toBeLessThan(
      installUpdate.mock.invocationCallOrder[0],
    )
    expect(screen.queryByText('安装并重启')).not.toBeInTheDocument()
  })

  it('shows 下载失败 and does not install when download fails', async () => {
    const user = userEvent.setup()
    const downloadUpdate = vi.fn().mockRejectedValue('sha mismatch')
    const installUpdate = vi.fn()
    render(
      <OneClickUpdateProbe
        downloadUpdate={downloadUpdate}
        installUpdate={installUpdate}
      />,
    )

    await user.click(screen.getByRole('button', { name: /下载并安装/ }))

    expect(await screen.findByText('下载失败: sha mismatch')).toBeInTheDocument()
    expect(screen.queryByText(/安装失败/)).not.toBeInTheDocument()
    expect(installUpdate).not.toHaveBeenCalled()
    expect(screen.getByRole('button', { name: /重试/ })).toBeInTheDocument()
  })

  it('shows 安装失败 when install fails after a successful download', async () => {
    const user = userEvent.setup()
    const downloadUpdate = vi.fn().mockResolvedValue('/tmp/beefex-desktop-win-x64.exe')
    const installUpdate = vi.fn().mockRejectedValue(new Error('NSIS installer exited 1'))
    render(
      <OneClickUpdateProbe
        downloadUpdate={downloadUpdate}
        installUpdate={installUpdate}
      />,
    )

    await user.click(screen.getByRole('button', { name: /下载并安装/ }))

    expect(await screen.findByText('安装失败: NSIS installer exited 1')).toBeInTheDocument()
    expect(screen.queryByText(/下载失败/)).not.toBeInTheDocument()
    expect(downloadUpdate).toHaveBeenCalledTimes(1)
    expect(installUpdate).toHaveBeenCalledTimes(1)
    expect(screen.getByRole('button', { name: /重试/ })).toBeInTheDocument()
  })
})

describe('inAppUpdateFailureText', () => {
  it('keeps download and install labels distinct in English', () => {
    expect(inAppUpdateFailureText('download', 'sha mismatch', {
      downloadFailed: 'Download failed',
      installFailed: 'Installation failed',
    })).toBe('Download failed: sha mismatch')
    expect(inAppUpdateFailureText('install', 'NSIS installer exited 1', {
      downloadFailed: 'Download failed',
      installFailed: 'Installation failed',
    })).toBe('Installation failed: NSIS installer exited 1')
  })
})
