import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { ScopedApprovalDialog } from './ScopedApprovalDialog'

describe('ScopedApprovalDialog', () => {
  it('starts on the safe rejection action and exposes dialog semantics', async () => {
    render(
      <ScopedApprovalDialog
        title="允许运行终端命令？"
        description="只对当前动作有效"
        approveLabel="允许"
        onApprove={vi.fn()}
        onReject={vi.fn()}
      />,
    )

    expect(screen.getByRole('dialog', { name: '允许运行终端命令？' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '拒绝' })).toHaveAttribute('data-slot', 'button')
    expect(screen.getByRole('button', { name: '允许' })).toHaveAttribute('data-slot', 'button')
    await waitFor(() => expect(screen.getByRole('button', { name: '拒绝', hidden: false })).toHaveFocus())
  })

  it('rejects on Escape and approves only from the explicit action', () => {
    const onApprove = vi.fn()
    const onReject = vi.fn()
    render(
      <ScopedApprovalDialog
        title="允许修改项目文件？"
        description="只对当前动作有效"
        approveLabel="允许"
        onApprove={onApprove}
        onReject={onReject}
      />,
    )

    fireEvent.keyDown(window, { key: 'Escape' })
    expect(onReject).toHaveBeenCalledTimes(1)
    expect(onApprove).not.toHaveBeenCalled()

    fireEvent.click(screen.getByRole('button', { name: '允许' }))
    expect(onApprove).toHaveBeenCalledTimes(1)
  })
})
