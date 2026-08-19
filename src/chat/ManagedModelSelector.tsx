import { useEffect, useMemo, useRef, useState } from 'react'
import { Check, ChevronDown, ChevronRight } from 'lucide-react'
import {
  groupManagedModels,
  managedModelDisplayName,
  type ManagedModelFamily,
} from './managedModelPresentation'

interface ManagedModelSelectorProps {
  models: readonly string[]
  value: string
  onChange: (model: string) => void
  placement?: 'up' | 'down'
}

export function ManagedModelSelector({ models, value, onChange, placement = 'down' }: ManagedModelSelectorProps) {
  const [open, setOpen] = useState(false)
  const [expanded, setExpanded] = useState<Record<ManagedModelFamily['id'], boolean>>({
    openai: false,
    anthropic: false,
    other: false,
  })
  const rootRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    if (!open) return
    const close = (event: MouseEvent) => {
      if (!rootRef.current?.contains(event.target as Node)) setOpen(false)
    }
    window.addEventListener('mousedown', close)
    return () => window.removeEventListener('mousedown', close)
  }, [open])

  const visible = useMemo(() => (models.length > 0 ? models : value ? [value] : []), [models, value])
  const families = useMemo(() => groupManagedModels(visible), [visible])

  const chooseModel = (model: string) => {
    onChange(model)
    setOpen(false)
  }

  const modelRow = (model: string) => (
    <button
      key={model}
      type="button"
      aria-label={`选择模型 ${model}`}
      onClick={() => chooseModel(model)}
      className="flex w-full items-center justify-between gap-3 rounded-md px-2 py-2 text-left text-[12px] text-[var(--beef-text)] hover:bg-[var(--beef-raised)]"
    >
      <span className="truncate">{managedModelDisplayName(model)}</span>
      {model === value && <Check size={13} className="shrink-0 text-[var(--beef-active)]" />}
    </button>
  )

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
        <div className={`beef-managed-model-popover chat-motion-popover absolute left-0 z-[220] min-w-56 overflow-hidden rounded-[10px] border border-[var(--beef-border)] p-1.5 shadow-xl ${placement === 'up' ? 'bottom-9' : 'top-9'}`}>
          <p className="px-2 py-1 text-[9px] font-semibold uppercase tracking-[0.12em] text-[var(--beef-text-secondary)]">BeefAPI 可用模型</p>
          <div
            data-testid="managed-model-scroll-region"
            className="max-h-[min(26rem,calc(100vh-8rem))] overflow-y-auto overscroll-contain pr-0.5"
          >
            {families.map((family, familyIndex) => (
              <section
                key={family.id}
                aria-label={`${family.label} 模型`}
                className={familyIndex > 0 ? 'mt-1.5 border-t border-[var(--beef-border)] pt-1.5' : ''}
              >
                <p className="px-2 py-1 text-[9px] font-semibold tracking-[0.13em] text-[var(--beef-text-secondary)]">
                  {family.label}
                </p>
                {family.featured.map(modelRow)}
                {family.secondary.length > 0 && (
                  <>
                    <button
                      type="button"
                      aria-expanded={expanded[family.id]}
                      onClick={() => setExpanded((current) => ({ ...current, [family.id]: !current[family.id] }))}
                      className="flex w-full items-center gap-1.5 rounded-md px-2 py-2 text-left text-[11px] text-[var(--beef-text-secondary)] hover:bg-[var(--beef-raised)] hover:text-[var(--beef-text)]"
                    >
                      <ChevronRight
                        size={12}
                        aria-hidden="true"
                        className={`shrink-0 transition-transform ${expanded[family.id] ? 'rotate-90' : ''}`}
                      />
                      <span>{family.secondaryLabel}</span>
                      <span className="ml-auto tabular-nums opacity-65">{family.secondary.length}</span>
                    </button>
                    {expanded[family.id] && family.secondary.map(modelRow)}
                  </>
                )}
              </section>
            ))}
            {visible.length === 0 && <p className="px-2 py-4 text-center text-[11px] text-[var(--beef-text-secondary)]">没有可用模型</p>}
          </div>
        </div>
      )}
    </div>
  )
}
