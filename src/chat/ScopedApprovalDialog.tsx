import { useEffect, useRef, type ReactNode } from 'react'
import { Wrench, X } from 'lucide-react'

interface ScopedApprovalDialogProps {
  title: string
  description: string
  approveLabel: string
  preview?: ReactNode
  onApprove: () => void
  onReject: () => void
}

export function ScopedApprovalDialog({
  title,
  description,
  approveLabel,
  preview,
  onApprove,
  onReject,
}: ScopedApprovalDialogProps) {
  const rejectRef = useRef<HTMLButtonElement>(null)
  const titleId = 'scoped-approval-title'

  useEffect(() => {
    rejectRef.current?.focus()
  }, [])

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') onReject()
    }
    window.addEventListener('keydown', handleKeyDown)
    return () => window.removeEventListener('keydown', handleKeyDown)
  }, [onReject])

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/20 px-4" data-tauri-drag-region="false">
      <section
        className="beef-approval-dialog w-full max-w-md rounded-[6px] border p-4 shadow-xl"
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
      >
        <div className="mb-3 flex items-start gap-2">
          <Wrench size={17} className="mt-0.5 shrink-0 text-[var(--beef-active)]" />
          <div className="min-w-0 flex-1">
            <h2 id={titleId} className="text-[14px] font-semibold text-neutral-900 dark:text-neutral-100">
              {title}
            </h2>
            <p className="mt-1 text-[12px] text-neutral-500 dark:text-neutral-400">
              {description}
            </p>
          </div>
          <button
            type="button"
            className="rounded-md p-1 text-neutral-400 hover:bg-neutral-100 hover:text-neutral-700 dark:hover:bg-neutral-800 dark:hover:text-neutral-200"
            aria-label="关闭并拒绝"
            onClick={onReject}
          >
            <X size={14} />
          </button>
        </div>
        {preview}
        <div className="flex justify-end gap-2">
          <button
            ref={rejectRef}
            type="button"
            className="rounded-md px-3 py-1.5 text-[12px] font-medium text-neutral-600 hover:bg-neutral-100 dark:text-neutral-300 dark:hover:bg-neutral-800"
            onClick={onReject}
          >
            拒绝
          </button>
          <button
            type="button"
            className="rounded-[5px] bg-[var(--beef-active)] px-3 py-1.5 text-[12px] font-medium text-[#FFF8F0] hover:bg-[var(--beef-pressed)] active:scale-[0.97]"
            onClick={onApprove}
          >
            {approveLabel}
          </button>
        </div>
      </section>
    </div>
  )
}
