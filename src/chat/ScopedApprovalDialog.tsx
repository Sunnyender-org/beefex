import { useEffect, useRef, type ReactNode } from 'react'
import { Wrench, X } from 'lucide-react'
import { Button } from '../bflabs/vendor/src/components/Button'

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
    <div className="beef-approval-backdrop fixed inset-0 z-50 flex items-center justify-center px-4" data-tauri-drag-region="false">
      <section
        className="beef-approval-dialog w-full max-w-lg rounded-[4px] border p-5"
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
            className="rounded-[2px] p-1 text-neutral-400 hover:bg-black/[0.05] hover:text-[var(--beef-text)] dark:hover:bg-white/[0.08]"
            aria-label="关闭并拒绝"
            onClick={onReject}
          >
            <X size={14} />
          </button>
        </div>
        {preview}
        <div className="flex justify-end gap-2">
          <Button
            ref={rejectRef}
            variant="quiet"
            size="sm"
            className="beef-approval-action beef-approval-action--reject"
            onClick={onReject}
          >
            拒绝
          </Button>
          <Button
            variant="accent"
            size="sm"
            className="beef-approval-action beef-approval-action--approve"
            onClick={onApprove}
          >
            {approveLabel}
          </Button>
        </div>
      </section>
    </div>
  )
}
