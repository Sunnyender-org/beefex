import { useEffect, useRef, useState } from 'react'
import { Check, ChevronDown } from 'lucide-react'

interface ManagedModelSelectorProps {
  models: readonly string[]
  value: string
  onChange: (model: string) => void
}

export function ManagedModelSelector({ models, value, onChange }: ManagedModelSelectorProps) {
  const [open, setOpen] = useState(false)
  const rootRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    if (!open) return
    const close = (event: MouseEvent) => {
      if (!rootRef.current?.contains(event.target as Node)) setOpen(false)
    }
    window.addEventListener('mousedown', close)
    return () => window.removeEventListener('mousedown', close)
  }, [open])

  const visible = models.length > 0 ? models : value ? [value] : []
  return (
    <div ref={rootRef} className="relative" data-tauri-drag-region="false">
      <button
        type="button"
        aria-label="BeefAPI 模型"
        aria-expanded={open}
        onClick={() => setOpen((current) => !current)}
        className="beef-managed-model"
      >
        <span className="beef-managed-model-mark" aria-hidden="true" />
        <span className="beef-managed-model-name">{value || '选择模型'}</span>
        <ChevronDown size={13} aria-hidden="true" />
      </button>
      {open && (
        <div className="chat-motion-popover absolute left-0 top-9 z-[220] min-w-56 overflow-hidden rounded-[10px] border border-[var(--beef-border)] bg-[var(--beef-surface)] p-1.5 shadow-xl">
          <p className="px-2 py-1 text-[9px] font-semibold uppercase tracking-[0.12em] text-[var(--beef-text-secondary)]">BeefAPI 可用模型</p>
          {visible.map((model) => (
            <button
              key={model}
              type="button"
              onClick={() => { onChange(model); setOpen(false) }}
              className="flex w-full items-center justify-between gap-3 rounded-md px-2 py-2 text-left text-[12px] text-[var(--beef-text)] hover:bg-[var(--beef-raised)]"
            >
              <span className="truncate">{model}</span>
              {model === value && <Check size={13} className="shrink-0 text-[var(--beef-active)]" />}
            </button>
          ))}
          {visible.length === 0 && <p className="px-2 py-4 text-center text-[11px] text-[var(--beef-text-secondary)]">没有可用模型</p>}
        </div>
      )}
    </div>
  )
}
