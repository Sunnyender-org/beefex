import { useEffect, useMemo, useState } from 'react'
import { CheckCircle2, Copy, ExternalLink, RefreshCw, RotateCcw, ShieldCheck, TerminalSquare } from 'lucide-react'
import { api, type CodexPluginPreview, type CodexPluginStatus } from '../api/tauri'
import { useBeefApiAccount } from '../beefapi/useBeefApiAccount'
import { Button } from '../components/Button'
import { copyToClipboard } from '../utils/clipboard'
import { Input, Select, SettingRow, SettingsGroup } from './components'

type Props = { lang: string }

const STATUS_CLASS: Record<CodexPluginStatus['state'], string> = {
  configured: 'ok',
  ready: 'accent',
  conflict: 'danger',
  failed: 'danger',
  missing: 'warn',
  unsupported: 'warn',
}

function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}

export function ClientIntegrationsPanel({ lang }: Props) {
  const { state: account } = useBeefApiAccount()
  const models = useMemo(() => {
    const values = account.allowedModels?.filter(Boolean) ?? []
    return [...new Set(values)]
  }, [account.allowedModels])
  const [status, setStatus] = useState<CodexPluginStatus | null>(null)
  const [model, setModel] = useState('')
  const [credential, setCredential] = useState('')
  const [preview, setPreview] = useState<CodexPluginPreview | null>(null)
  const [busy, setBusy] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [notice, setNotice] = useState<string | null>(null)
  const [rollbackArmed, setRollbackArmed] = useState(false)

  const refresh = async () => {
    const next = await api.codexPluginInspect()
    setStatus(next)
    setModel((current) => current || next.configuredModel || account.defaultModel || models[0] || '')
    return next
  }

  useEffect(() => {
    let disposed = false
    void api.codexPluginInspect()
      .then((next) => {
        if (disposed) return
        setStatus(next)
        setModel(next.configuredModel || account.defaultModel || models[0] || '')
      })
      .catch((caught) => {
        if (!disposed) setError(message(caught))
      })
    return () => { disposed = true }
  }, [account.defaultModel, models])

  const run = async (operation: string, action: () => Promise<void>) => {
    setBusy(operation)
    setError(null)
    setNotice(null)
    try {
      await action()
    } catch (caught) {
      setError(message(caught))
    } finally {
      setBusy(null)
    }
  }

  const previewChanges = () => run('preview', async () => {
    const next = await api.codexPluginPreview(model)
    setPreview(next)
    setNotice(lang === 'zh' ? '已生成变更预览，尚未写入。' : 'Preview ready; nothing has been written.')
  })
  const apply = async () => {
    await run('apply', async () => {
      const result = await api.codexPluginApply(model, credential)
      setPreview(null)
      setStatus(result.status)
      setNotice(lang === 'zh' ? 'Codex BeefAPI 配置已写入并读回校验。' : 'Codex BeefAPI profile was written and read back.')
    })
    setCredential('')
  }
  const verify = () => run('verify', async () => {
    const result = await api.codexPluginVerify()
    setStatus(result.status)
    setNotice(lang === 'zh' ? 'Codex strict-config 检查通过。' : 'Codex strict-config check passed.')
  })
  const rollback = () => {
    if (!rollbackArmed) {
      setRollbackArmed(true)
      setNotice(lang === 'zh' ? '再次点击“确认回滚”才会修改文件。' : 'Click “Confirm rollback” to modify files.')
      return Promise.resolve()
    }
    return run('rollback', async () => {
      const result = await api.codexPluginRollback()
      setStatus(result.status)
      setPreview(null)
      setRollbackArmed(false)
      setNotice(lang === 'zh' ? '已回滚 Beefex 管理的 Codex 配置。' : 'Beefex-managed Codex configuration was rolled back.')
    })
  }

  const signedIn = account.phase === 'signed_in'
  const canConfigure = signedIn
    && models.length > 0
    && Boolean(model)
    && status?.supported
    && (status.state === 'ready' || status.state === 'configured')
    && !busy
  const launchCommand = status?.launchCommand || 'codex --profile beefapi'

  return (
    <>
      <SettingsGroup title={lang === 'zh' ? '第一方客户端插件' : 'First-party client plugin'}>
        <div className="kv-panel">
          <div className="flex items-start justify-between gap-3">
            <div>
              <div className="kv-panel-title"><TerminalSquare /> Codex × BeefAPI</div>
              <div className="kv-panel-body">
                {lang === 'zh'
                  ? '为 Codex 安装独立的 BeefAPI profile。不会修改 Codex 主配置，也不会读取 Beefex 登录凭据。'
                  : 'Install an isolated BeefAPI profile for Codex. It does not modify the main Codex config or read the Beefex login credential.'}
              </div>
            </div>
            {status && <span className={`kv-tag ${STATUS_CLASS[status.state]}`}>{status.state}</span>}
          </div>
        </div>

        <SettingRow
          label={lang === 'zh' ? 'Codex 环境' : 'Codex environment'}
          description={status?.codexVersion
            ? `${status.codexVersion} · ${status.profilePath}`
            : (lang === 'zh' ? '需要安装受支持版本的 Codex。' : 'A supported Codex version is required.')}
        >
          <Button size="sm" onClick={() => void run('refresh', async () => { await refresh() })} disabled={Boolean(busy)}>
            <RefreshCw size={11} className={busy === 'refresh' ? 'animate-spin' : ''} />
            {lang === 'zh' ? '检测' : 'Detect'}
          </Button>
        </SettingRow>

        <SettingRow
          label={lang === 'zh' ? '模型' : 'Model'}
          description={signedIn
            ? (lang === 'zh' ? '只显示当前 BeefAPI 账号允许的模型。' : 'Only models allowed for the current BeefAPI account are shown.')
            : (lang === 'zh' ? '请先登录 BeefAPI。' : 'Sign in to BeefAPI first.')}
        >
          <Select
            className="w-52"
            value={model}
            onChange={setModel}
            options={models.map((value) => ({ value, label: value }))}
          />
        </SettingRow>

        <SettingRow
          label={lang === 'zh' ? '独立 API Token' : 'Separate API token'}
          description={lang === 'zh'
            ? '在 BeefAPI 创建给 Codex 使用的 API Token；仅保存到 Beefex owner-only 文件。'
            : 'Create a BeefAPI API token for Codex; it is stored only in a Beefex owner-only file.'}
        >
          <div className="w-64">
            <Input
              type="password"
              value={credential}
              onChange={setCredential}
              placeholder={lang === 'zh' ? '不会写入配置或回执' : 'Never written to config or receipts'}
              autoComplete="off"
            />
          </div>
        </SettingRow>

        <div className="flex flex-wrap items-center justify-end gap-2 py-3">
          <Button size="sm" onClick={() => void previewChanges()} disabled={!canConfigure}>
            {lang === 'zh' ? '预览变更' : 'Preview changes'}
          </Button>
          <Button variant="primary" size="sm" onClick={() => void apply()} disabled={!canConfigure || credential.trim().length < 8}>
            <ShieldCheck size={11} />
            {busy === 'apply' ? (lang === 'zh' ? '配置中' : 'Applying') : (lang === 'zh' ? '配置 Codex' : 'Configure Codex')}
          </Button>
          <Button size="sm" onClick={() => void verify()} disabled={status?.state !== 'configured' || Boolean(busy)}>
            <CheckCircle2 size={11} /> {lang === 'zh' ? '验证' : 'Verify'}
          </Button>
          <Button variant="danger" size="sm" onClick={() => void rollback()} disabled={status?.state !== 'configured' || Boolean(busy)}>
            <RotateCcw size={11} /> {rollbackArmed
              ? (lang === 'zh' ? '确认回滚' : 'Confirm rollback')
              : (lang === 'zh' ? '回滚' : 'Rollback')}
          </Button>
        </div>
      </SettingsGroup>

      {preview && (
        <SettingsGroup title={lang === 'zh' ? '变更预览' : 'Change preview'}>
          <div className="kv-panel info">
            <div className="kv-panel-body">{preview.credentialContract}</div>
            <div className="mt-2 space-y-1.5">
              {preview.changes.map((change) => (
                <div key={`${change.action}:${change.path}`} className="text-[11px] text-neutral-600 dark:text-neutral-300">
                  <span className="kv-tag accent mr-2">{change.action}</span>
                  <span className="font-mono break-all">{change.path}</span>
                </div>
              ))}
            </div>
            <pre className="kv-textarea mono mt-3 max-h-64 overflow-auto whitespace-pre-wrap break-all" data-testid="codex-profile-preview">
              {preview.configPreview}
            </pre>
          </div>
        </SettingsGroup>
      )}

      {(notice || error || status?.reason) && (
        <div className={`kv-panel ${error || status?.state === 'failed' || status?.state === 'conflict' ? 'warn' : 'info'}`}>
          <div className="kv-panel-body break-all">{error || notice || status?.reason}</div>
        </div>
      )}

      <div className="mt-3 flex items-center gap-1.5 text-[11px] text-neutral-500">
        <ExternalLink size={11} />
        <span>{lang === 'zh' ? '启动命令：' : 'Launch command: '}</span>
        <code>{launchCommand}</code>
        <Button
          size="sm"
          variant="ghost"
          onClick={() => void run('copy', async () => {
            if (!await copyToClipboard(launchCommand)) throw new Error('codex_plugin_copy_failed')
            setNotice(lang === 'zh' ? '启动命令已复制。' : 'Launch command copied.')
          })}
          disabled={Boolean(busy)}
        >
          <Copy size={11} /> {lang === 'zh' ? '复制' : 'Copy'}
        </Button>
      </div>
    </>
  )
}
