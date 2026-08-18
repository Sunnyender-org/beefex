import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { CompletionReceipt } from './CompletionReceipt'

describe('CompletionReceipt', () => {
  it('uses the canonical BFLabs success tag without changing receipt data', () => {
    render(
      <CompletionReceipt
        lang="zh"
        receipt={{
          changed_files: [{ path: 'src/example.ts', additions: 2, removals: 1 }],
          commands: [],
          validations: [],
        }}
      />,
    )

    const status = screen.getByText('完成回执')
    expect(status).toHaveAttribute('data-slot', 'status-tag')
    expect(status).toHaveClass('bf-status-tag--success')
    expect(screen.getByText('src/example.ts')).toBeInTheDocument()
  })
})
