import type { ChatTaskStatus } from './types'

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

export function TaskStatusBadge({ status, lang }: { status: ChatTaskStatus; lang: 'zh' | 'en' }) {
  return (
    <div
      className={`beef-task-status flex h-6 shrink-0 items-center gap-1.5 rounded-[2px] border px-2 text-[10px] font-bold tracking-[0.04em] ${styles[status]}`}
      data-task-status={status}
      title={labels[status][lang]}
    >
      <span
        className={`h-1.5 w-1.5 rounded-full bg-current ${status === 'running' ? 'animate-pulse' : ''}`}
        aria-hidden="true"
      />
      <span>{labels[status][lang]}</span>
    </div>
  )
}
