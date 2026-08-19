import { CheckCircle2, FileCode2, SquareTerminal } from 'lucide-react'
import { StatusTag } from '../bflabs/vendor/src/components/StatusTag'
import type { ChatCompletionReceipt } from './types'

export function CompletionReceipt({
  receipt,
  lang,
}: {
  receipt: ChatCompletionReceipt
  lang: 'zh' | 'en'
}) {
  const files = receipt.changed_files ?? receipt.changedFiles ?? []
  const commands = receipt.commands ?? []
  if (files.length === 0 && commands.length === 0) return null

  return (
    <details className="beef-completion-receipt mx-auto mb-2 w-[min(100%-2rem,52rem)] shrink-0 rounded-[4px] border text-[12px]">
      <summary className="flex cursor-pointer list-none items-center gap-2 px-3 py-2 font-medium marker:hidden">
        <CheckCircle2 size={14} className="text-[var(--beef-verified)]" />
        <StatusTag tone="success" className="beef-completion-receipt__status">
          {lang === 'zh' ? '完成回执' : 'Completion receipt'}
        </StatusTag>
        <span className="font-normal text-neutral-400">
          {files.length > 0 && `${files.length} ${lang === 'zh' ? '个文件' : 'files'}`}
          {files.length > 0 && commands.length > 0 && ' · '}
          {commands.length > 0 && `${commands.length} ${lang === 'zh' ? '条命令' : 'commands'}`}
        </span>
      </summary>
      <div className="grid gap-2 border-t border-[var(--beef-border)] px-3 py-2.5 sm:grid-cols-2">
        {files.length > 0 && (
          <div className="min-w-0 space-y-1.5">
            <div className="flex items-center gap-1.5 text-[11px] font-medium text-neutral-500 dark:text-neutral-400">
              <FileCode2 size={12} />
              {lang === 'zh' ? '实际变更' : 'Observed changes'}
            </div>
            {files.map((file) => (
              <div key={file.path} className="flex min-w-0 items-center justify-between gap-2 font-mono text-[11px]">
                <span className="truncate">{file.path}</span>
                <span className="shrink-0 text-neutral-400">+{file.additions} -{file.removals}</span>
              </div>
            ))}
          </div>
        )}
        {commands.length > 0 && (
          <div className="min-w-0 space-y-1.5">
            <div className="flex items-center gap-1.5 text-[11px] font-medium text-neutral-500 dark:text-neutral-400">
              <SquareTerminal size={12} />
              {lang === 'zh' ? '命令与验证' : 'Commands and checks'}
            </div>
            {commands.map((command, index) => (
              <div key={`${command.command}-${index}`} className="min-w-0 font-mono text-[11px]">
                <div className="flex items-center gap-2">
                  <span className="truncate">{command.command}</span>
                  <span className={command.exit_status === 0 || command.exitStatus === 0 ? 'text-[var(--beef-verified)]' : 'text-red-600 dark:text-red-400'}>
                    {lang === 'zh' ? '退出' : 'exit'} {command.exit_status ?? command.exitStatus ?? '?'}
                  </span>
                </div>
                <div className="truncate text-neutral-400">{command.cwd}</div>
              </div>
            ))}
          </div>
        )}
      </div>
    </details>
  )
}
