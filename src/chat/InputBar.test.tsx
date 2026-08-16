import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { InputBar } from './InputBar'

describe('InputBar', () => {
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
})
