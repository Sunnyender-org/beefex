import { describe, expect, it } from 'vitest'
import { isCurrentConversationAwaitingApproval, resolveTaskStatus } from './taskStatus'

describe('resolveTaskStatus', () => {
  it('prioritizes scoped approval over running and persisted terminal state', () => {
    expect(resolveTaskStatus({
      awaitingApproval: true,
      running: true,
      failed: false,
      persisted: 'completed',
    })).toBe('awaiting_approval')
  })

  it('uses live running/error state before the last persisted state', () => {
    expect(resolveTaskStatus({
      awaitingApproval: false,
      running: true,
      failed: false,
      persisted: 'completed',
    })).toBe('running')
    expect(resolveTaskStatus({
      awaitingApproval: false,
      running: false,
      failed: true,
      persisted: 'completed',
    })).toBe('failed')
  })

  it('falls back to persisted state and then idle', () => {
    expect(resolveTaskStatus({
      awaitingApproval: false,
      running: false,
      failed: false,
      persisted: 'interrupted',
    })).toBe('interrupted')
    expect(resolveTaskStatus({
      awaitingApproval: false,
      running: false,
      failed: false,
      persisted: null,
    })).toBe('idle')
  })
})

describe('isCurrentConversationAwaitingApproval', () => {
  it('does not treat two missing ids as a real approval match', () => {
    expect(isCurrentConversationAwaitingApproval(undefined, undefined)).toBe(false)
    expect(isCurrentConversationAwaitingApproval(null, null)).toBe(false)
  })

  it('matches only the exact pending task', () => {
    expect(isCurrentConversationAwaitingApproval('task-a', 'task-a')).toBe(true)
    expect(isCurrentConversationAwaitingApproval('task-a', 'task-b')).toBe(false)
  })
})
