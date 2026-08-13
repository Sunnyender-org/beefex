import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { PiProjectTrustDialog } from './PiProjectTrustDialog'

const project = {
  id: 'proj_test',
  name: 'Beefex',
  root_path: '/tmp/beefex/apps/desktop',
  created_at: 1,
  updated_at: 1,
}

const preview = {
  requestedPath: '/tmp/beefex/apps/desktop',
  trustPath: '/tmp/beefex',
  isGitRepository: true,
  decision: 'unknown' as const,
  inheritedFrom: null,
  resources: ['.pi/skills'],
}

describe('PiProjectTrustDialog', () => {
  it('explains canonical repo trust without merging tool approval', () => {
    render(<PiProjectTrustDialog project={project} preview={preview} onTrust={vi.fn()} onCancel={vi.fn()} />)
    expect(screen.getByText('/tmp/beefex')).toBeInTheDocument()
    expect(screen.getByText(/设置、扩展、Skills、提示词和项目包/)).toBeInTheDocument()
    expect(screen.getByText(/修改文件、运行命令或访问网络仍会按动作单独请求/)).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '信任并打开' })).toBeInTheDocument()
  })

  it('supports explicit trust and Escape cancellation', () => {
    const onTrust = vi.fn()
    const onCancel = vi.fn()
    render(<PiProjectTrustDialog project={project} preview={preview} onTrust={onTrust} onCancel={onCancel} />)
    fireEvent.click(screen.getByRole('button', { name: '信任并打开' }))
    fireEvent.keyDown(window, { key: 'Escape' })
    expect(onTrust).toHaveBeenCalledOnce()
    expect(onCancel).toHaveBeenCalledOnce()
  })
})
