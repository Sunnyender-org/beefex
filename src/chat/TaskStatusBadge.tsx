import type { ChatTaskStatus } from './types'
import { StatusTag, type StatusTagTone } from '../bflabs/vendor/src/components/StatusTag'

const styles: Record<ChatTaskStatus, string> = {
  idle: 'beef-task-status--idle',
  running: 'beef-task-status--running',
  awaiting_approval: 'beef-task-status--approval',
  completed: 'beef-task-status--completed',
  failed: 'beef-task-status--failed',
  cancelled: 'beef-task-status--cancelled',
  interrupted: 'beef-task-status--interrupted',
}

const labels: Record<ChatTaskStatus, { zh: string; en: string }> = {
  idle: { zh: '待命', en: 'Idle' },
  running: { zh: '正在执行', en: 'Running' },
  awaiting_approval: { zh: '待确认', en: 'Approval needed' },
  completed: { zh: '已完成', en: 'Completed' },
  failed: { zh: '失败', en: 'Failed' },
  cancelled: { zh: '已取消', en: 'Cancelled' },
  interrupted: { zh: '已中断', en: 'Interrupted' },
}

const tones: Record<ChatTaskStatus, StatusTagTone> = {
  idle: 'neutral',
  running: 'progress',
  awaiting_approval: 'accent',
  completed: 'success',
  failed: 'neutral',
  cancelled: 'neutral',
  interrupted: 'accent',
}

export function TaskStatusBadge({ status, lang }: { status: ChatTaskStatus; lang: 'zh' | 'en' }) {
  return (
    <StatusTag
      className={`beef-task-status shrink-0 border ${styles[status]}`}
      tone={tones[status]}
      showDot={false}
      data-task-status={status}
      title={labels[status][lang]}
    >
      <span
        className={`h-1.5 w-1.5 rounded-full bg-current ${status === 'running' ? 'animate-pulse' : ''}`}
        aria-hidden="true"
      />
      <span>{labels[status][lang]}</span>
    </StatusTag>
  )
}
