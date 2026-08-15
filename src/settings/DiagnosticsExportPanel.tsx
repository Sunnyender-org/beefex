import { useState } from 'react'
import { save } from '@tauri-apps/plugin-dialog'
import { Check, Download, Eye, ShieldCheck } from 'lucide-react'
import { api, type DiagnosticsExportReceipt, type DiagnosticsPreview } from '../api/tauri'
import { Button } from '../components/Button'

const CATEGORY_LABELS: Record<string, { zh: string; en: string }> = {
  startup: { zh: '启动', en: 'Startup' },
  account_transition: { zh: '登录状态', en: 'Account state' },
  pi_child_lifecycle: { zh: 'Pi 进程', en: 'Pi process' },
  run_terminal: { zh: '任务终态', en: 'Run terminal state' },
  task_recovery: { zh: '任务恢复', en: 'Task recovery' },
  renderer_error: { zh: '界面错误', en: 'Renderer errors' },
  rust_panic: { zh: '应用崩溃', en: 'Application panics' },
  export_lifecycle: { zh: '导出状态', en: 'Export state' },
}

const EXCLUDED_LABELS: Record<string, { zh: string; en: string }> = {
  credentials: { zh: '凭据', en: 'credentials' },
  account_identifiers: { zh: '账号标识', en: 'account identifiers' },
  prompts_and_transcripts: { zh: '提示词与对话', en: 'prompts and transcripts' },
  tool_payloads: { zh: '工具参数与结果', en: 'tool payloads' },
  project_content: { zh: '项目与文件内容', en: 'project and file content' },
  absolute_paths: { zh: '绝对路径', en: 'absolute paths' },
  request_and_response_bodies: { zh: '请求与响应正文', en: 'request and response bodies' },
  raw_pi_events: { zh: '原始 Pi 事件', en: 'raw Pi events' },
}

function localizedLabel(key: string, lang: string, labels: typeof CATEGORY_LABELS): string {
  const label = labels[key]
  return label ? (lang === 'zh' ? label.zh : label.en) : key.replaceAll('_', ' ')
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
}

function formatTimeRange(first: string | null, last: string | null, lang: string): string | null {
  if (!first || !last) return null
  const locale = lang === 'zh' ? 'zh-CN' : 'en-US'
  const format = (value: string) => new Intl.DateTimeFormat(locale, {
    month: 'short',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
  }).format(new Date(value))
  return first === last ? format(first) : `${format(first)} → ${format(last)}`
}

export function DiagnosticsExportPanel({ lang }: { lang: string }) {
  const zh = lang === 'zh'
  const [preview, setPreview] = useState<DiagnosticsPreview | null>(null)
  const [receipt, setReceipt] = useState<DiagnosticsExportReceipt | null>(null)
  const [status, setStatus] = useState<'idle' | 'loading' | 'ready' | 'exporting' | 'success' | 'error'>('idle')
  const [error, setError] = useState<string | null>(null)

  const loadPreview = async () => {
    setStatus('loading')
    setError(null)
    setReceipt(null)
    try {
      setPreview(await api.previewDiagnosticsExport())
      setStatus('ready')
    } catch {
      setStatus('error')
      setError(zh ? '暂时无法读取本地诊断信息。' : 'Local diagnostics are temporarily unavailable.')
    }
  }

  const exportArchive = async () => {
    const date = new Date().toISOString().slice(0, 10)
    let path: string | null
    try {
      path = await save({
        title: zh ? '导出 Beefex 诊断包' : 'Export Beefex diagnostics',
        defaultPath: `beefex-diagnostics-${date}.zip`,
        filters: [{ name: 'ZIP', extensions: ['zip'] }],
      })
    } catch {
      setStatus('error')
      setError(zh ? '无法选择导出位置。' : 'Could not choose an export location.')
      return
    }
    if (!path) return
    setStatus('exporting')
    setError(null)
    try {
      const nextReceipt = await api.exportDiagnostics(path)
      setReceipt(nextReceipt)
      setStatus('success')
    } catch {
      setStatus('error')
      setError(zh ? '导出失败，没有留下不完整文件。' : 'Export failed. No partial archive was kept.')
    }
  }

  return (
    <div className="kv-panel">
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <div className="kv-panel-title flex items-center gap-2">
            <ShieldCheck size={14} strokeWidth={1.8} />
            {zh ? '本地诊断' : 'Local diagnostics'}
          </div>
          <p className="kv-panel-body mt-1">
            {zh
              ? '仅在你确认后导出经过净化的应用状态，用于定位测试版问题。'
              : 'Exports sanitized application state only after you confirm, for troubleshooting test builds.'}
          </p>
        </div>
        {!preview && (
          <Button size="sm" onClick={loadPreview} disabled={status === 'loading'}>
            <Eye size={12} />
            {status === 'loading' ? (zh ? '读取中' : 'Loading') : (zh ? '查看包含项' : 'Preview contents')}
          </Button>
        )}
      </div>

      {preview && (
        <div className="mt-3 border-t border-[color:var(--kv-border)] pt-3">
          <div className="flex flex-wrap gap-1.5" aria-label={zh ? '包含类别' : 'Included categories'}>
            {(preview.categories.length > 0 ? preview.categories : ['startup']).map((category) => (
              <span key={category} className="kv-tag">
                {localizedLabel(category, lang, CATEGORY_LABELS)}
              </span>
            ))}
          </div>
          <p className="kv-row-desc mt-2">
            {zh ? '明确排除：' : 'Explicitly excluded: '}
            {preview.excludedCategories
              .map((category) => localizedLabel(category, lang, EXCLUDED_LABELS))
              .join(zh ? '、' : ', ')}
          </p>
          <div className="mt-3 flex flex-wrap items-center justify-between gap-3">
            <div className="min-w-0">
              <div className="kv-row-desc tabular-nums">
                {preview.fileCount} {zh ? '个本地文件' : 'local files'} · {formatBytes(preview.approximateBytes)} · v{preview.appVersion}
              </div>
              {formatTimeRange(preview.firstTimestamp, preview.lastTimestamp, lang) && (
                <div className="kv-row-desc mt-1 tabular-nums">
                  {zh ? '时间范围：' : 'Time range: '}
                  {formatTimeRange(preview.firstTimestamp, preview.lastTimestamp, lang)}
                </div>
              )}
            </div>
            <Button
              size="sm"
              variant="primary"
              onClick={exportArchive}
              disabled={status === 'exporting'}
            >
              <Download size={12} />
              {status === 'exporting' ? (zh ? '导出中' : 'Exporting') : (zh ? '导出诊断包' : 'Export diagnostics')}
            </Button>
          </div>
        </div>
      )}

      {status === 'success' && receipt && (
        <div className="mt-3 flex items-center gap-2 text-[12px] text-emerald-700 dark:text-emerald-400" role="status">
          <Check size={13} />
          <span>{zh ? '诊断包已导出' : 'Diagnostics exported'} · {formatBytes(receipt.archiveBytes)}</span>
        </div>
      )}
      {status === 'error' && error && (
        <div className="kv-inline-error mt-3" role="alert">{error}</div>
      )}
    </div>
  )
}
