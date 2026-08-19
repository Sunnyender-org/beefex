import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { TaskStatusBadge } from './TaskStatusBadge'

describe('TaskStatusBadge', () => {
  it('adapts task state to the canonical BFLabs status primitive', () => {
    render(<TaskStatusBadge status="running" lang="en" />)

    const status = screen.getByTitle('Running')
    expect(status).toHaveAttribute('data-slot', 'status-tag')
    expect(status).toHaveAttribute('data-task-status', 'running')
    expect(status).toHaveClass('bf-status-tag--progress')
  })
})
