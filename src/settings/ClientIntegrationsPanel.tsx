import { useEffect, useMemo, useState } from 'react'
import { CheckCircle2, RefreshCw, RotateCcw, Sparkles, TerminalSquare } from 'lucide-react'
import { api, type ManagedClientsStatus } from '../api/tauri'
import { useBeefApiAccount } from '../beefapi/useBeefApiAccount'
import { Button } from '../components/Button'
import { Select, SettingRow, SettingsGroup } from './components'

type Props = { lang: string }

const LABELS: Record<string, string> = {
  codex: 'Codex',
  image2: 'BeefAPI Image2',
  'claude-code': 'Claude Code',
  'claude-desktop': 'Claude Desktop',
  grok: 'Grok',
}

function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}

export function ClientIntegrationsPanel({ lang }: Props) {
  const { state: account } = useBeefApiAccount()
  const models = useMemo(() => [...new Set(account.allowedModels?.filter(Boolean) ?? [])], [account.allowedModels])
  const [status, setStatus] = useState<ManagedClientsStatus | null>(null)
  const [model, setModel] = useState('')
  const [busy, setBusy] = useState<string | null>(null)
  const [notice, setNotice] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [rollbackArmed, setRollbackArmed] = useState(false)

  const refresh = async () => {
    const next = await api.managedClientsInspect()
    setStatus(next)
    setModel((current) => current || account.defaultModel || models[0] || '')
  }

  useEffect(() => {
    let disposed = false
    void api.managedClientsInspect().then((next) => {
      if (disposed) return
      setStatus(next)
      setModel(account.defaultModel || models[0] || '')
    }).catch((caught) => {
      if (!disposed) setError(message(caught))
    })
    return () => { disposed = true }
  }, [account.defaultModel, models])

  const run = async (operation: string, action: () => Promise<void>) => {
    setBusy(operation)
    setError(null)
    setNotice(null)
    try { await action() } catch (caught) { setError(message(caught)) } finally { setBusy(null) }
  }

  const apply = () => run('apply', async () => {
    const result = await api.managedClientsApply(model)
    setStatus(result.status)
    setNotice(lang === 'zh'
      ? '已用当前 Beefex 登录完成 Codex、Image2、Claude 与 Grok 配置。'
      : 'Codex, Image2, Claude, and Grok were configured from the current Beefex sign-in.')
  })

  const verify = () => run('verify', async () => {
    const result = await api.managedClientsVerify()
    setStatus(result.status)
    setNotice(lang === 'zh' ? '本地配置、读回与版本检查通过。' : 'Local config, readback, and version checks passed.')
  })

  const rollback = () => {
    if (!rollbackArmed) {
      setRollbackArmed(true)
      setNotice(lang === 'zh' ? '再次点击确认回滚。' : 'Click once more to confirm rollback.')
      return
    }
    void run('rollback', async () => {
      const result = await api.managedClientsRollback()
      setStatus(result.status)
      setRollbackArmed(false)
      setNotice(lang === 'zh' ? '已恢复 Beefex 配置前的文件。' : 'Files were restored to their pre-Beefex state.')
    })
  }

  const canConfigure = account.phase === 'signed_in' && Boolean(model) && !busy

  return (
    <>
      <SettingsGroup title={lang === 'zh' ? 'BeefAPI 客户端接入' : 'BeefAPI client setup'}>
        <div className="kv-panel">
          <div className="flex items-start justify-between gap-3">
            <div>
              <div className="kv-panel-title"><TerminalSquare /> {lang === 'zh' ? '一键配置 Coding Agents' : 'Configure coding agents'}</div>
              <div className="kv-panel-body">
                {lang === 'zh'
                  ? '复用当前 Beefex 登录，一次配置 Codex、BeefAPI Image2、Claude Code、Claude Desktop 和 Grok。无需粘贴 Key、额外登录或 CC Switch。'
                  : 'Reuse the current Beefex sign-in for Codex, BeefAPI Image2, Claude Code, Claude Desktop, and Grok. No pasted keys, extra login, or CC Switch.'}
              </div>
            </div>
            {status && <span className={`kv-tag ${status.state === 'configured' ? 'ok' : status.state === 'partial' ? 'warn' : 'accent'}`}>{status.state}</span>}
          </div>
        </div>

        <SettingRow
          label={lang === 'zh' ? 'Codex 默认模型' : 'Codex default model'}
          description={account.phase === 'signed_in'
            ? (lang === 'zh' ? '仅使用当前账号允许的模型；Claude 与 Grok 使用各自固定分组。' : 'Only account-allowed models; Claude and Grok use their fixed groups.')
            : (lang === 'zh' ? '请先登录 BeefAPI。' : 'Sign in to BeefAPI first.')}
        >
          <Select className="w-52" value={model} onChange={setModel} options={models.map((value) => ({ value, label: value }))} />
        </SettingRow>

        <div className="grid grid-cols-1 gap-2 py-3 sm:grid-cols-2">
          {(status?.clients ?? []).map((client) => (
            <div key={client.id} className="kv-panel flex items-center justify-between gap-2">
              <div>
                <div className="kv-panel-title">{LABELS[client.id] ?? client.id}</div>
                <div className="kv-panel-body font-mono">{client.launchCommand}</div>
              </div>
              <span className={`kv-tag ${client.configured ? 'ok' : client.detected ? 'accent' : 'warn'}`}>
                {client.configured ? (lang === 'zh' ? '已配置' : 'configured') : client.detected ? (lang === 'zh' ? '已检测' : 'detected') : (lang === 'zh' ? '未检测' : 'missing')}
              </span>
            </div>
          ))}
        </div>

        <div className="flex flex-wrap items-center justify-end gap-2 py-3">
          <Button size="sm" onClick={() => void run('refresh', refresh)} disabled={Boolean(busy)}>
            <RefreshCw size={11} className={busy === 'refresh' ? 'animate-spin' : ''} /> {lang === 'zh' ? '检测' : 'Detect'}
          </Button>
          <Button variant="primary" size="sm" onClick={() => void apply()} disabled={!canConfigure}>
            <Sparkles size={11} /> {busy === 'apply' ? (lang === 'zh' ? '配置中' : 'Configuring') : (lang === 'zh' ? '全部配置好' : 'Configure all')}
          </Button>
          <Button size="sm" onClick={() => void verify()} disabled={status?.state !== 'configured' || Boolean(busy)}>
            <CheckCircle2 size={11} /> {lang === 'zh' ? '本地配置检查' : 'Local checks'}
          </Button>
          <Button variant="danger" size="sm" onClick={rollback} disabled={status?.state === 'ready' || Boolean(busy)}>
            <RotateCcw size={11} /> {rollbackArmed ? (lang === 'zh' ? '确认回滚' : 'Confirm rollback') : (lang === 'zh' ? '回滚' : 'Rollback')}
          </Button>
        </div>
      </SettingsGroup>

      {(notice || error || status?.reason) && (
        <div className={`kv-panel ${error ? 'warn' : 'info'}`}>
          <div className="kv-panel-body break-all">{error || notice || status?.reason}</div>
        </div>
      )}
    </>
  )
}
