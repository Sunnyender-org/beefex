import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { PiTaskMenu } from './PiTaskMenu'

describe('PiTaskMenu', () => {
  it('opens from the Task bar and dispatches typed Pi commands', async () => {
    const onRun = vi.fn().mockResolvedValue({ messages: 3 })
    render(<PiTaskMenu onRun={onRun} />)
    fireEvent.click(screen.getByRole('button', { name: 'Pi Task 命令' }))
    fireEvent.click(screen.getByRole('button', { name: /会话统计/ }))
    expect(onRun).toHaveBeenCalledWith({ type: 'get_session_stats' })
    expect(await screen.findByText(/"messages": 3/)).toBeInTheDocument()
  })

  it('supports the command palette shortcut and filtering', () => {
    render(<PiTaskMenu onRun={vi.fn()} />)
    fireEvent.keyDown(window, { key: 'P', metaKey: true, shiftKey: true })
    const search = screen.getByRole('textbox', { name: '搜索 Pi Task 命令' })
    fireEvent.change(search, { target: { value: '压缩' } })
    expect(screen.getByRole('button', { name: /压缩上下文/ })).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /会话统计/ })).not.toBeInTheDocument()
  })

  it('collects values for typed session commands without a browser prompt', () => {
    const onRun = vi.fn().mockResolvedValue({})
    render(<PiTaskMenu onRun={onRun} />)
    fireEvent.click(screen.getByRole('button', { name: 'Pi Task 命令' }))
    fireEvent.click(screen.getByRole('button', { name: /重命名 Pi 会话/ }))
    fireEvent.change(screen.getByRole('textbox', { name: '会话名称' }), { target: { value: '登录修复' } })
    fireEvent.click(screen.getByRole('button', { name: '执行' }))
    expect(onRun).toHaveBeenCalledWith({ type: 'set_session_name', name: '登录修复' })
  })
})
