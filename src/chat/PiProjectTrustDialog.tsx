import { useEffect, useRef } from 'react'
import { createPortal } from 'react-dom'
import { FolderKey, X } from 'lucide-react'
import type { ChatProject, PiProjectTrustPreview } from './types'

interface PiProjectTrustDialogProps {
  project: ChatProject
  preview: PiProjectTrustPreview
  saving?: boolean
  error?: string
  onTrust: () => void
  onCancel: () => void
}

export function PiProjectTrustDialog({
  project,
  preview,
  saving = false,
  error = '',
  onTrust,
  onCancel,
}: PiProjectTrustDialogProps) {
  const cancelRef = useRef<HTMLButtonElement>(null)

  useEffect(() => {
    cancelRef.current?.focus()
  }, [])

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape' && !saving) onCancel()
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [onCancel, saving])

  return createPortal(
    <div
      className="chat-motion-fade fixed inset-0 z-[320] flex items-center justify-center bg-black/30 px-4 backdrop-blur-[1px]"
      data-tauri-drag-region="false"
    >
      <section
        className="chat-motion-modal-in w-full max-w-[440px] rounded-[10px] border border-neutral-200 bg-white p-5 shadow-xl dark:border-neutral-700 dark:bg-[#252527]"
        role="dialog"
        aria-modal="true"
        aria-labelledby="pi-project-trust-title"
      >
        <div className="flex items-start gap-3">
          <div className="mt-0.5 rounded-lg bg-[var(--beef-active)]/10 p-2 text-[var(--beef-active)]">
            <FolderKey size={18} strokeWidth={1.8} />
          </div>
          <div className="min-w-0 flex-1">
            <h2 id="pi-project-trust-title" className="text-[15px] font-semibold text-neutral-900 dark:text-neutral-50">
              信任“{project.name}”中的内容？
            </h2>
            <p className="mt-1.5 text-[12px] leading-5 text-neutral-500 dark:text-neutral-400">
              信任后，Pi 可以加载这个项目里的设置、扩展、Skills、提示词和项目包。其中的代码可能会运行。
            </p>
          </div>
          <button
            type="button"
            aria-label="取消"
            disabled={saving}
            onClick={onCancel}
            className="rounded-md p-1 text-neutral-400 hover:bg-neutral-100 hover:text-neutral-700 disabled:opacity-40 dark:hover:bg-neutral-800 dark:hover:text-neutral-200"
          >
            <X size={15} />
          </button>
        </div>

        <div className="mt-4 rounded-lg border border-neutral-200 bg-neutral-50 px-3 py-2.5 dark:border-neutral-700 dark:bg-neutral-900/60">
          <div className="text-[11px] font-medium uppercase tracking-[0.08em] text-neutral-400">
            {preview.isGitRepository ? 'Git 仓库根目录' : '项目文件夹'}
          </div>
          <div className="mt-1 break-all font-mono text-[11px] leading-4 text-neutral-700 dark:text-neutral-300">
            {preview.trustPath}
          </div>
        </div>

        <p className="mt-3 text-[12px] leading-5 text-neutral-600 dark:text-neutral-300">
          这只决定是否加载项目内容。修改文件、运行命令或访问网络仍会按动作单独请求你的允许。
        </p>
        {error && <p role="alert" className="mt-2 text-[12px] text-red-600 dark:text-red-400">{error}</p>}

        <div className="mt-5 flex justify-end gap-2">
          <button
            ref={cancelRef}
            type="button"
            disabled={saving}
            onClick={onCancel}
            className="rounded-lg px-3 py-1.5 text-[13px] text-neutral-600 hover:bg-black/[0.04] disabled:opacity-40 dark:text-neutral-300 dark:hover:bg-white/[0.06]"
          >
            取消
          </button>
          <button
            type="button"
            disabled={saving}
            onClick={onTrust}
            className="rounded-lg bg-neutral-900 px-3 py-1.5 text-[13px] font-medium text-white hover:bg-neutral-800 disabled:opacity-50 dark:bg-neutral-100 dark:text-neutral-950 dark:hover:bg-white"
          >
            {saving ? '正在保存…' : '信任并打开'}
          </button>
        </div>
      </section>
    </div>,
    document.body,
  )
}
