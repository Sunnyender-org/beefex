import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { ManagedModelSelector } from './ManagedModelSelector'
import { resolveManagedModelValue } from './managedModelPolicy'

describe('ManagedModelSelector', () => {
  it('keeps an allowed draft selection before the first Task exists', () => {
    expect(resolveManagedModelValue('gpt-5.6-terra', 'gpt-5.6-sol', ['gpt-5.6-sol', 'gpt-5.6-terra']))
      .toBe('gpt-5.6-terra')
    expect(resolveManagedModelValue('not-allowed', 'gpt-5.6-sol', ['gpt-5.6-sol', 'gpt-5.6-terra']))
      .toBe('gpt-5.6-sol')
  })

  it('shows only server-allowed model ids and no group or provider routing', () => {
    const onChange = vi.fn()
    render(<ManagedModelSelector models={['gpt-5.6-sol', 'claude-opus-4.8']} value="gpt-5.6-sol" onChange={onChange} />)
    fireEvent.click(screen.getByRole('button', { name: 'BeefAPI 模型' }))
    expect(screen.getByRole('button', { name: /claude-opus-4.8/ })).toBeInTheDocument()
    expect(screen.queryByText(/gpt-pro|group|route/i)).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: /claude-opus-4.8/ }))
    expect(onChange).toHaveBeenCalledWith('claude-opus-4.8')
  })
})
