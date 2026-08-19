import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { ManagedModelSelector } from './ManagedModelSelector'
import { groupManagedModels } from './managedModelPresentation'
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
    render(<ManagedModelSelector models={['gpt-5.6-sol', 'claude-fable-5']} value="gpt-5.6-sol" onChange={onChange} />)
    fireEvent.click(screen.getByRole('button', { name: 'BeefAPI 模型' }))
    expect(screen.getByRole('button', { name: '选择模型 claude-fable-5' })).toBeInTheDocument()
    expect(screen.queryByText(/gpt-pro|group|route/i)).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: '选择模型 claude-fable-5' }))
    expect(onChange).toHaveBeenCalledWith('claude-fable-5')
  })

  it('groups featured models and keeps other family models collapsed by default', () => {
    render(
      <ManagedModelSelector
        models={[
          'gpt-5.5',
          'claude-opus-4.8',
          'claude-sonnet-5',
          'gpt-5.6-terra',
          'claude-fable-5',
          'gpt-5.6-sol',
          'claude-opus-5',
        ]}
        value="gpt-5.6-sol"
        onChange={vi.fn()}
      />,
    )

    fireEvent.click(screen.getByRole('button', { name: 'BeefAPI 模型' }))

    const openai = screen.getByRole('region', { name: 'OPENAI 模型' })
    const anthropic = screen.getByRole('region', { name: 'ANTHROPIC 模型' })
    expect(openai.compareDocumentPosition(anthropic) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
    expect(screen.getByRole('button', { name: '选择模型 gpt-5.6-terra' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '选择模型 gpt-5.6-sol' })).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '选择模型 gpt-5.5' })).not.toBeInTheDocument()

    const anthropicFeatured = ['claude-fable-5', 'claude-opus-5', 'claude-sonnet-5']
      .map((model) => screen.getByRole('button', { name: `选择模型 ${model}` }))
    expect(anthropicFeatured[0].compareDocumentPosition(anthropicFeatured[1]) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
    expect(anthropicFeatured[1].compareDocumentPosition(anthropicFeatured[2]) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
    expect(screen.queryByRole('button', { name: '选择模型 claude-opus-4.8' })).not.toBeInTheDocument()

    fireEvent.click(screen.getByRole('button', { name: /其他 OpenAI 模型/ }))
    expect(screen.getByRole('button', { name: '选择模型 gpt-5.5' })).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: /其他 Anthropic 模型/ }))
    expect(screen.getByRole('button', { name: '选择模型 claude-opus-4.8' })).toBeInTheDocument()
  })

  it('uses a bounded wheel-scroll region and never invents unavailable featured models', () => {
    render(<ManagedModelSelector models={['gpt-5.6-sol', 'claude-fable-5']} value="gpt-5.6-sol" onChange={vi.fn()} />)
    fireEvent.click(screen.getByRole('button', { name: 'BeefAPI 模型' }))
    expect(screen.getByTestId('managed-model-scroll-region')).toHaveClass('overflow-y-auto', 'overscroll-contain')
    expect(screen.queryByRole('button', { name: '选择模型 claude-opus-5' })).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '选择模型 claude-sonnet-5' })).not.toBeInTheDocument()
  })

  it('preserves the backend order within GPT-5.6 and ranks the three featured Claude families', () => {
    expect(groupManagedModels([
      'claude-sonnet-5',
      'gpt-5.6-terra',
      'claude-fable-5',
      'gpt-5.6-sol',
      'claude-opus-5',
    ])).toEqual([
      {
        id: 'openai',
        label: 'OPENAI',
        featured: ['gpt-5.6-terra', 'gpt-5.6-sol'],
        secondary: [],
        secondaryLabel: '其他 OpenAI 模型',
      },
      {
        id: 'anthropic',
        label: 'ANTHROPIC',
        featured: ['claude-fable-5', 'claude-opus-5', 'claude-sonnet-5'],
        secondary: [],
        secondaryLabel: '其他 Anthropic 模型',
      },
    ])
  })

  it('opens upward when embedded in the footer composer', () => {
    render(
      <ManagedModelSelector
        models={['gpt-5.6-sol']}
        value="gpt-5.6-sol"
        onChange={vi.fn()}
        placement="up"
      />,
    )

    fireEvent.click(screen.getByRole('button', { name: 'BeefAPI 模型' }))
    expect(screen.getByText('BeefAPI 可用模型').parentElement).toHaveClass('bottom-9')
  })
})
