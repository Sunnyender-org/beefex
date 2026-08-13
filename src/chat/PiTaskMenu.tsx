import { useEffect, useMemo, useRef, useState } from 'react'
import { Command, LoaderCircle, Search, X } from 'lucide-react'
import { PI_TASK_MENU_ACTIONS, type PiRpcCommand } from './piCapabilities'
import type { PiTaskMenuAction } from './piCapabilities'

interface PiTaskMenuProps {
  disabled?: boolean
  onRun: (command: PiRpcCommand) => Promise<unknown>
}

export function PiTaskMenu({ disabled = false, onRun }: PiTaskMenuProps) {
  const [open, setOpen] = useState(false)
  const [query, setQuery] = useState('')
  const [running, setRunning] = useState('')
  const [result, setResult] = useState('')
  const [inputAction, setInputAction] = useState<PiTaskMenuAction | null>(null)
  const [inputValue, setInputValue] = useState('')
  const inputRef = useRef<HTMLInputElement>(null)
  const rootRef = useRef<HTMLDivElement>(null)
  const actions = useMemo(() => {
    const needle = query.trim().toLowerCase()
    if (!needle) return PI_TASK_MENU_ACTIONS
    return PI_TASK_MENU_ACTIONS.filter((action) =>
      `${action.label} ${action.description} ${action.command?.type ?? action.id}`.toLowerCase().includes(needle),
    )
  }, [query])

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && event.shiftKey && event.key.toLowerCase() === 'p') {
        event.preventDefault()
        if (!disabled) setOpen((value) => !value)
      } else if (event.key === 'Escape') {
        setOpen(false)
      }
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [disabled])

  useEffect(() => {
    if (!open) return
    inputRef.current?.focus()
    const onPointerDown = (event: MouseEvent) => {
      if (!rootRef.current?.contains(event.target as Node)) setOpen(false)
    }
    window.addEventListener('mousedown', onPointerDown)
    return () => window.removeEventListener('mousedown', onPointerDown)
  }, [open])

  const execute = async (id: string, command: PiRpcCommand) => {
    if (running) return
    setRunning(id)
    setResult('')
    try {
      const data = await onRun(command)
      const text = typeof data === 'string' ? data : JSON.stringify(data, null, 2)
      setResult(text.length > 2400 ? `${text.slice(0, 2400)}\n…` : text || '完成')
    } catch (error) {
      setResult(error instanceof Error ? error.message : String(error))
    } finally {
      setRunning('')
    }
  }

  const choose = (action: PiTaskMenuAction) => {
    if (action.input) {
      setInputAction(action)
      setInputValue('')
      setResult('')
      return
    }
    if (action.command) void execute(action.id, action.command)
  }

  const submitInput = () => {
    const value = inputValue.trim()
    const input = inputAction?.input
    if (!value || !input) return
    const action = inputAction
    setInputAction(null)
    void execute(action.id, input.build(value))
  }

  return (
    <div ref={rootRef} className="relative" data-tauri-drag-region="false">
      <button
        type="button"
        disabled={disabled}
        aria-label="Pi Task 命令"
        aria-expanded={open}
        onClick={() => setOpen((value) => !value)}
        className="inline-flex h-7 items-center gap-1.5 rounded-md border border-[var(--beef-border)] bg-[var(--beef-raised)] px-2 text-[11px] font-medium text-[var(--beef-text-secondary)] transition-colors hover:text-[var(--beef-text)] disabled:cursor-default disabled:opacity-40"
      >
        <Command size={13} strokeWidth={1.8} />
        Pi
      </button>
      {open && (
        <div className="chat-motion-popover absolute left-0 top-9 z-[220] w-[340px] overflow-hidden rounded-[10px] border border-[var(--beef-border)] bg-[var(--beef-surface)] shadow-2xl">
          <div className="flex items-center gap-2 border-b border-[var(--beef-border)] px-3 py-2">
            <Search size={14} className="shrink-0 text-[var(--beef-text-secondary)]" />
            <input
              ref={inputRef}
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              placeholder="搜索 Pi Task 命令"
              aria-label="搜索 Pi Task 命令"
              className="min-w-0 flex-1 bg-transparent text-[12px] text-[var(--beef-text)] outline-none placeholder:text-[var(--beef-text-secondary)]"
            />
            <button type="button" aria-label="关闭 Pi Task 命令" onClick={() => setOpen(false)} className="rounded p-1 text-[var(--beef-text-secondary)] hover:bg-[var(--beef-raised)]">
              <X size={13} />
            </button>
          </div>
          <div className="custom-scrollbar max-h-[310px] overflow-y-auto p-1.5">
            {actions.map((action) => (
              <button
                key={action.id}
                type="button"
                disabled={Boolean(running)}
                onClick={() => choose(action)}
                className="flex w-full items-start gap-2.5 rounded-md px-2.5 py-2 text-left hover:bg-[var(--beef-raised)] disabled:opacity-50"
              >
                <span className="mt-0.5 flex h-5 w-5 shrink-0 items-center justify-center rounded bg-[var(--beef-active)]/10 text-[10px] font-semibold text-[var(--beef-active)]">
                  {running === action.id ? <LoaderCircle size={12} className="animate-spin" /> : 'π'}
                </span>
                <span className="min-w-0">
                  <span className="block text-[12px] font-medium text-[var(--beef-text)]">{action.label}</span>
                  <span className="mt-0.5 block text-[10px] leading-4 text-[var(--beef-text-secondary)]">{action.description}</span>
                </span>
              </button>
            ))}
            {actions.length === 0 && <p className="px-3 py-6 text-center text-[11px] text-[var(--beef-text-secondary)]">没有匹配的命令</p>}
          </div>
          {inputAction?.input && (
            <form className="border-t border-[var(--beef-border)] p-2.5" onSubmit={(event) => { event.preventDefault(); submitInput() }}>
              <label htmlFor="pi-task-command-value" className="mb-1 block text-[10px] font-medium text-[var(--beef-text-secondary)]">{inputAction.input.label}</label>
              <div className="flex gap-1.5">
                <input
                  id="pi-task-command-value"
                  autoFocus
                  value={inputValue}
                  onChange={(event) => setInputValue(event.target.value)}
                  placeholder={inputAction.input.placeholder}
                  className="min-w-0 flex-1 rounded-md border border-[var(--beef-border)] bg-[var(--beef-raised)] px-2 py-1.5 text-[11px] text-[var(--beef-text)] outline-none focus:border-[var(--beef-active)]"
                />
                <button type="submit" disabled={!inputValue.trim()} className="rounded-md bg-[var(--beef-active)] px-2.5 text-[10px] font-medium text-white disabled:opacity-40">执行</button>
              </div>
            </form>
          )}
          {result && (
            <pre className="custom-scrollbar max-h-36 overflow-auto border-t border-[var(--beef-border)] bg-[var(--beef-raised)] px-3 py-2 text-[10px] leading-4 text-[var(--beef-text)]">{result}</pre>
          )}
          <div className="border-t border-[var(--beef-border)] px-3 py-1.5 text-[9px] text-[var(--beef-text-secondary)]">⌘⇧P 打开 · Escape 关闭</div>
        </div>
      )}
    </div>
  )
}
