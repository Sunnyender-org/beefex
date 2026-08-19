import { createRef, type MutableRefObject } from 'react'
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { InputBar } from './InputBar'

describe('InputBar', () => {
  it('keeps the managed model selector inside the composer', () => {
    const onManagedModelChange = vi.fn()
    render(
      <InputBar
        onSend={vi.fn()}
        managedModels={['gpt-5.6-sol', 'claude-fable-5']}
        managedModel="gpt-5.6-sol"
        onManagedModelChange={onManagedModelChange}
      />,
    )

    const selector = screen.getByRole('button', { name: 'BeefAPI 模型' })
    expect(screen.getByRole('textbox')).toHaveAttribute('rows', '2')
    expect(screen.getByRole('textbox')).toHaveClass('min-h-[52px]')
    expect(selector.closest('[data-chat-composer="true"]')).not.toBeNull()
    fireEvent.click(selector)
    expect(screen.getByText('BeefAPI 可用模型').parentElement).toHaveClass('bottom-9')
    fireEvent.click(screen.getByRole('button', { name: '选择模型 claude-fable-5' }))
    expect(onManagedModelChange).toHaveBeenCalledWith('claude-fable-5')
  })

  it('allows only one send submission while the current submission is pending', async () => {
    let finishFirst: (() => void) | undefined
    const first = new Promise<void>((resolve) => {
      finishFirst = resolve
    })
    const onSend = vi.fn()
      .mockReturnValueOnce(first)
      .mockResolvedValueOnce(undefined)

    render(<InputBar onSend={onSend} />)
    const input = screen.getByRole('textbox')
    fireEvent.change(input, { target: { value: 'configure clients' } })
    const send = screen.getByRole('button', { name: '发送' })

    fireEvent.click(send)
    fireEvent.click(send)
    expect(onSend).toHaveBeenCalledTimes(1)

    await act(async () => finishFirst?.())
    fireEvent.change(input, { target: { value: 'next request' } })
    fireEvent.click(screen.getByRole('button', { name: '发送' }))
    await waitFor(() => expect(onSend).toHaveBeenCalledTimes(2))
  })

  it('keeps the send lock when the composer remounts during the first submission', async () => {
    let finishFirst: (() => void) | undefined
    const first = new Promise<void>((resolve) => {
      finishFirst = resolve
    })
    const onSend = vi.fn().mockReturnValue(first)
    const submissionLockRef = createRef<boolean>() as MutableRefObject<boolean>
    submissionLockRef.current = false
    const view = render(
      <InputBar key="welcome" onSend={onSend} submissionLockRef={submissionLockRef} />,
    )

    fireEvent.change(screen.getByRole('textbox'), { target: { value: 'configure clients' } })
    fireEvent.click(screen.getByRole('button', { name: '发送' }))
    expect(onSend).toHaveBeenCalledTimes(1)

    view.rerender(
      <InputBar key="conversation" onSend={onSend} submissionLockRef={submissionLockRef} />,
    )
    fireEvent.change(screen.getByRole('textbox'), { target: { value: 'configure clients' } })
    fireEvent.click(screen.getByRole('button', { name: '发送' }))
    expect(onSend).toHaveBeenCalledTimes(1)

    await act(async () => finishFirst?.())
  })
})
