import type { ChatTaskStatus } from './types'

export function resolveTaskStatus(input: {
  awaitingApproval: boolean
  running: boolean
  failed: boolean
  persisted: ChatTaskStatus | null | undefined
}): ChatTaskStatus {
  if (input.awaitingApproval) return 'awaiting_approval'
  if (input.running) return 'running'
  if (input.failed) return 'failed'
  return input.persisted ?? 'idle'
}

export function isCurrentConversationAwaitingApproval(
  pendingConversationId: string | null | undefined,
  currentConversationId: string | null | undefined,
) {
  return Boolean(
    pendingConversationId
    && currentConversationId
    && pendingConversationId === currentConversationId,
  )
}
