// @vitest-environment jsdom

import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { DiagnosticsExportPanel } from './DiagnosticsExportPanel'

const mocks = vi.hoisted(() => ({
  previewDiagnosticsExport: vi.fn(),
  exportDiagnostics: vi.fn(),
  save: vi.fn(),
}))

vi.mock('../api/tauri', () => ({
  api: {
    previewDiagnosticsExport: mocks.previewDiagnosticsExport,
    exportDiagnostics: mocks.exportDiagnostics,
  },
}))

vi.mock('@tauri-apps/plugin-dialog', () => ({ save: mocks.save }))

describe('DiagnosticsExportPanel', () => {
  beforeEach(() => {
    mocks.previewDiagnosticsExport.mockReset()
    mocks.exportDiagnostics.mockReset()
    mocks.save.mockReset()
  })

  it('previews categories before an intentional export and shows the receipt', async () => {
    mocks.previewDiagnosticsExport.mockResolvedValue({
      categories: ['startup', 'renderer_error'],
      excludedCategories: ['credentials', 'project_content'],
      fileCount: 2,
      approximateBytes: 2048,
      appVersion: '0.1.0',
      firstTimestamp: '2026-08-15T12:00:00Z',
      lastTimestamp: '2026-08-15T12:05:00Z',
      skippedRecords: 0,
    })
    mocks.save.mockResolvedValue('/tmp/beefex-diagnostics.zip')
    mocks.exportDiagnostics.mockResolvedValue({
      path: '/tmp/beefex-diagnostics.zip',
      archiveBytes: 1024,
      inventory: ['manifest.json', 'events/events-00.ndjson'],
      manifestSchemaVersion: 1,
    })
    const user = userEvent.setup()
    render(<DiagnosticsExportPanel lang="zh" />)

    await user.click(screen.getByRole('button', { name: '查看包含项' }))
    expect(await screen.findByText('启动')).toBeInTheDocument()
    expect(screen.getByText(/明确排除/)).toBeInTheDocument()
    expect(screen.getByText(/凭据/)).toBeInTheDocument()
    expect(screen.getByText(/时间范围/)).toBeInTheDocument()
    expect(screen.queryByText(/原始日志内容/)).not.toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: '导出诊断包' }))
    expect(mocks.save).toHaveBeenCalledWith(expect.objectContaining({
      defaultPath: expect.stringMatching(/beefex-diagnostics.*\.zip$/),
    }))
    expect(mocks.exportDiagnostics).toHaveBeenCalledWith('/tmp/beefex-diagnostics.zip')
    expect(await screen.findByRole('status')).toHaveTextContent('诊断包已导出 · 1.0 KB')
  })

  it('keeps cancellation quiet and leaves no success state', async () => {
    mocks.previewDiagnosticsExport.mockResolvedValue({
      categories: [],
      excludedCategories: ['credentials'],
      fileCount: 0,
      approximateBytes: 0,
      appVersion: '0.1.0',
      firstTimestamp: null,
      lastTimestamp: null,
      skippedRecords: 0,
    })
    mocks.save.mockResolvedValue(null)
    const user = userEvent.setup()
    render(<DiagnosticsExportPanel lang="zh" />)

    await user.click(screen.getByRole('button', { name: '查看包含项' }))
    await user.click(await screen.findByRole('button', { name: '导出诊断包' }))

    expect(mocks.exportDiagnostics).not.toHaveBeenCalled()
    expect(screen.queryByText('诊断包已导出')).not.toBeInTheDocument()
  })

  it('keeps the preview and export actions keyboard reachable', async () => {
    mocks.previewDiagnosticsExport.mockResolvedValue({
      categories: ['startup'],
      excludedCategories: ['credentials'],
      fileCount: 1,
      approximateBytes: 512,
      appVersion: '0.1.0',
      firstTimestamp: null,
      lastTimestamp: null,
      skippedRecords: 0,
    })
    const user = userEvent.setup()
    render(<DiagnosticsExportPanel lang="zh" />)

    await user.tab()
    expect(screen.getByRole('button', { name: '查看包含项' })).toHaveFocus()
    await user.keyboard('{Enter}')

    const exportButton = await screen.findByRole('button', { name: '导出诊断包' })
    await user.tab()
    expect(exportButton).toHaveFocus()
  })

  it('shows a bounded error when archive creation fails', async () => {
    mocks.previewDiagnosticsExport.mockResolvedValue({
      categories: ['startup'],
      excludedCategories: ['credentials'],
      fileCount: 1,
      approximateBytes: 512,
      appVersion: '0.1.0',
      firstTimestamp: null,
      lastTimestamp: null,
      skippedRecords: 0,
    })
    mocks.save.mockResolvedValue('/tmp/beefex-diagnostics.zip')
    mocks.exportDiagnostics.mockRejectedValue(new Error('raw private failure'))
    const user = userEvent.setup()
    render(<DiagnosticsExportPanel lang="zh" />)

    await user.click(screen.getByRole('button', { name: '查看包含项' }))
    await user.click(await screen.findByRole('button', { name: '导出诊断包' }))

    expect(await screen.findByRole('alert')).toHaveTextContent('导出失败，没有留下不完整文件。')
    expect(screen.queryByText(/raw private failure/)).not.toBeInTheDocument()
  })
})
