import { useCallback, useEffect, useMemo, useState } from 'react'
import {
  ArrowRight,
  Check,
  CheckCircle2,
  Clipboard,
  CloudOff,
  ExternalLink,
  KeyRound,
  LoaderCircle,
  LockKeyhole,
  RefreshCw,
  ShieldCheck,
  XCircle,
} from 'lucide-react'
import { api, type BeefApiAccountState, type Settings } from '../api/tauri'
import { getSettingsCached, saveSettingsCached } from '../api/settingsCache'
import { Button } from '../components/Button'
import { useBeefApiAccount } from '../beefapi/useBeefApiAccount'
import {
  accountPresentation,
  formatAuthorizationRemaining,
  hasBeefApiAccountMetadata,
  type BeefApiAccountAction,
} from '../beefapi/accountPresentation'
import { copyToClipboard } from '../utils/clipboard'
import type { Lang } from '../settings/i18n'
import { usesNativeTitlebar } from '../chat/platform'

type OnboardingShellProps = {
  onComplete: () => void
  onSettingsChange?: () => void
}

function detectSystemLang(): Lang {
  const locale = (
    (typeof navigator !== 'undefined' && (navigator.language || navigator.languages?.[0])) || ''
  ).toLowerCase()
  return locale.startsWith('zh') ? 'zh' : 'en'
}

export function OnboardingShell({ onComplete, onSettingsChange }: OnboardingShellProps) {
  const account = useBeefApiAccount()
  const [settings, setSettings] = useState<Settings | null>(null)
  const [settingsError, setSettingsError] = useState<string | null>(null)
  const [saving, setSaving] = useState(false)
  const [nowSeconds, setNowSeconds] = useState(() => Math.floor(Date.now() / 1_000))
  const [copied, setCopied] = useState(false)
  const lang = (settings?.settingsLanguage || detectSystemLang()) as Lang
  const zh = lang === 'zh'
  const presentation = accountPresentation(account.state)
  const hasAccountReadback = hasBeefApiAccountMetadata(account.state)
  const copy = useMemo(() => accountCopy(account.state, lang), [account.state, lang])
  const remaining = formatAuthorizationRemaining(account.state.expiresAt, nowSeconds)
  const busy = account.operation !== null || saving

  const loadSettings = useCallback(async () => {
    setSettingsError(null)
    try {
      const loaded = await getSettingsCached()
      setSettings({
        ...loaded,
        settingsLanguage: loaded.settingsLanguage || detectSystemLang(),
      })
    } catch (error) {
      setSettingsError(error instanceof Error ? error.message : String(error))
    }
  }, [])

  useEffect(() => {
    void loadSettings()
  }, [loadSettings])

  useEffect(() => {
    if (!presentation.showVerification) return
    const timer = window.setInterval(
      () => setNowSeconds(Math.floor(Date.now() / 1_000)),
      1_000,
    )
    return () => window.clearInterval(timer)
  }, [presentation.showVerification])

  useEffect(() => {
    setCopied(false)
  }, [account.state.userCode])

  const enterWorkbench = useCallback(async () => {
    if (!settings || account.state.phase !== 'signed_in') return
    setSaving(true)
    setSettingsError(null)
    try {
      const saved = await saveSettingsCached({
        ...settings,
        onboardingStatus: 'completed',
      })
      setSettings(saved)
      onSettingsChange?.()
      onComplete()
    } catch (error) {
      setSettingsError(error instanceof Error ? error.message : String(error))
    } finally {
      setSaving(false)
    }
  }, [account.state.phase, onComplete, onSettingsChange, settings])

  const runPrimaryAction = useCallback(async (action: BeefApiAccountAction) => {
    switch (action) {
      case 'start':
      case 'retry':
        await account.startAuthorization()
        return
      case 'reopen':
        await account.reopenBrowser()
        return
      case 'continue':
        await enterWorkbench()
        return
      case 'reconnect':
        await account.reconnect()
        return
      case 'manage_account':
        await api.openExternal('https://beefapi.com/console')
        return
      case 'manage_tokens':
        await api.openExternal('https://beefapi.com/console/token')
        return
      case 'none':
        return
    }
  }, [account, enterWorkbench])

  const copyVerificationCode = useCallback(async () => {
    if (!account.state.userCode) return
    const success = await copyToClipboard(account.state.userCode)
    setCopied(success)
  }, [account.state.userCode])

  if (!settings && !settingsError) {
    return (
      <div className="beef-login-shell beef-login-shell--loading" aria-busy="true">
        <LoaderCircle size={18} className="animate-spin" />
        <span>{zh ? '正在准备 Beefex' : 'Preparing Beefex'}</span>
      </div>
    )
  }

  return (
    <main
      className={`beef-login-shell${usesNativeTitlebar ? ' beef-login-shell--native' : ''}`}
      lang={lang}
    >
      <aside className="beef-login-rail" data-tauri-drag-region>
        <div className="beef-wordmark" data-tauri-drag-region>
          <span className="beef-wordmark-mark" aria-hidden="true">B</span>
          <span>Beefex</span>
        </div>
        <div className="beef-login-rail-copy">
          <span className="beef-eyebrow">BEEFAPI CODING AGENT</span>
          <h1>{zh ? '登录，然后直接开始写代码。' : 'Sign in, then start coding.'}</h1>
          <p>
            {zh
              ? '账号、GPT Pro 模型和计费由 BeefAPI 管理。Beefex 不要求你配置 Provider、Base URL 或 API Key。'
              : 'BeefAPI manages your account, GPT Pro model, and billing. Beefex never asks you to configure a provider, base URL, or API key.'}
          </p>
        </div>
        <ol className="beef-login-route" aria-label={zh ? '开始使用步骤' : 'Getting started'}>
          {[
            zh ? '连接 BeefAPI' : 'Connect BeefAPI',
            zh ? '打开本地项目' : 'Open a local project',
            zh ? '提交 coding task' : 'Send a coding task',
          ].map((label, index) => (
            <li key={label} className={index === 0 ? 'is-current' : ''}>
              <span>{presentation.mode === 'ready' && index === 0 ? <Check size={12} /> : index + 1}</span>
              {label}
            </li>
          ))}
        </ol>
        <div className="beef-login-security">
          <ShieldCheck size={15} />
          <span>
            {zh
              ? '登录在系统浏览器完成，凭据只由 Beefex 后台安全存储。'
              : 'Authentication happens in your system browser. The credential stays in Beefex backend storage.'}
          </span>
        </div>
      </aside>

      <section className="beef-login-stage" data-tauri-drag-region>
        <div className="beef-login-panel" data-tauri-drag-region="false">
          <div className={`beef-login-status-icon is-${presentation.mode}`} aria-hidden="true">
            <AccountStateIcon mode={presentation.mode} />
          </div>
          <div className="beef-login-heading">
            <span className="beef-login-kicker">{copy.kicker}</span>
            <h2>{copy.title}</h2>
            <p>{copy.description}</p>
          </div>

          {presentation.showVerification && account.state.userCode ? (
            <div className="beef-verification-block">
              <div className="beef-verification-label">
                <span>{zh ? '一次性验证码' : 'One-time code'}</span>
                {remaining ? (
                  <span className="beef-verification-timer">
                    {zh ? `${remaining} 后过期` : `Expires in ${remaining}`}
                  </span>
                ) : null}
              </div>
              <div className="beef-verification-code" aria-label={account.state.userCode}>
                {account.state.userCode}
              </div>
              <div className="beef-verification-actions">
                <Button size="sm" onClick={() => void copyVerificationCode()} disabled={busy}>
                  <Clipboard size={13} />
                  {copied ? (zh ? '已复制' : 'Copied') : (zh ? '复制验证码' : 'Copy code')}
                </Button>
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={() => void account.reopenBrowser()}
                  disabled={busy}
                >
                  <ExternalLink size={13} />
                  {zh ? '重新打开浏览器' : 'Reopen browser'}
                </Button>
              </div>
              <p className="beef-login-trust-note">
                <LockKeyhole size={13} />
                {zh
                  ? '只在 beefapi.com 核对此验证码。Beefex 不会要求你粘贴 API Key。'
                  : 'Verify this code only on beefapi.com. Beefex will never ask you to paste an API key.'}
              </p>
            </div>
          ) : null}

          {hasAccountReadback ? (
            <dl className="beef-account-readback">
              <div>
                <dt>{zh ? '账号' : 'Account'}</dt>
                <dd>{account.state.email || (zh ? '已验证账号' : 'Verified account')}</dd>
              </div>
              <div>
                <dt>{zh ? '分组' : 'Group'}</dt>
                <dd>{formatGroup(account.state.group, lang)}</dd>
              </div>
              <div>
                <dt>{zh ? '默认模型' : 'Default model'}</dt>
                <dd>{formatModel(account.state.defaultModel, lang)}</dd>
              </div>
            </dl>
          ) : null}

          {(account.error || settingsError) ? (
            <div className="beef-login-error" role="alert">
              <XCircle size={14} />
              <span>
                {zh ? 'Beefex 无法完成此操作，请重试。' : 'Beefex could not complete this action. Try again.'}
              </span>
            </div>
          ) : null}

          <div className="beef-login-actions">
            <Button
              variant="primary"
              onClick={() => void runPrimaryAction(presentation.primaryAction)}
              disabled={busy || presentation.primaryAction === 'none' || !settings}
            >
              {busy ? <LoaderCircle size={14} className="animate-spin" /> : <PrimaryActionIcon action={presentation.primaryAction} />}
              {busy ? (zh ? '处理中' : 'Working') : primaryActionLabel(presentation.primaryAction, lang)}
              {!busy && presentation.primaryAction === 'continue' ? <ArrowRight size={14} /> : null}
            </Button>
            {presentation.canCancel ? (
              <Button
                variant="ghost"
                onClick={() => void account.cancelAuthorization()}
                disabled={busy}
              >
                {zh ? '取消' : 'Cancel'}
              </Button>
            ) : null}
          </div>
        </div>
      </section>
    </main>
  )
}

function AccountStateIcon({ mode }: { mode: ReturnType<typeof accountPresentation>['mode'] }) {
  if (mode === 'loading' || mode === 'waiting') return <LoaderCircle size={19} className="animate-spin" />
  if (mode === 'ready') return <CheckCircle2 size={20} />
  if (mode === 'offline') return <CloudOff size={20} />
  if (mode === 'retry' || mode === 'blocked' || mode === 'cleanup') return <XCircle size={20} />
  return <KeyRound size={20} />
}

function PrimaryActionIcon({ action }: { action: BeefApiAccountAction }) {
  if (action === 'reconnect' || action === 'retry') return <RefreshCw size={14} />
  if (action === 'manage_account' || action === 'manage_tokens' || action === 'reopen') {
    return <ExternalLink size={14} />
  }
  if (action === 'continue') return null
  return <KeyRound size={14} />
}

function accountCopy(state: BeefApiAccountState, lang: Lang) {
  const zh = lang === 'zh'
  if (state.phase === 'signed_out' && state.reason === 'initializing') {
    return {
      kicker: zh ? '正在检查本机账号' : 'Checking local account',
      title: zh ? '正在准备 Beefex' : 'Preparing Beefex',
      description: zh ? '正在核对 BeefAPI 账号与本地登录状态。' : 'Checking your BeefAPI account and local sign-in state.',
    }
  }
  switch (state.phase) {
    case 'signed_out':
      return {
        kicker: zh ? '账号连接' : 'Account connection',
        title: zh ? '登录 BeefAPI' : 'Sign in to BeefAPI',
        description: zh ? '将打开系统浏览器。无需配置 Provider、Base URL 或 API Key。' : 'Your system browser will open. No provider, base URL, or API key setup is required.',
      }
    case 'authorizing':
    case 'polling':
      return {
        kicker: zh ? '等待浏览器确认' : 'Waiting for browser confirmation',
        title: zh ? '在浏览器中确认登录' : 'Confirm sign-in in your browser',
        description: zh ? 'Beefex 正在等待 BeefAPI 完成授权与账号验证。' : 'Beefex is waiting for BeefAPI authorization and account validation.',
      }
    case 'signed_in':
      return {
        kicker: zh ? '账号已验证' : 'Account verified',
        title: zh ? 'BeefAPI 已连接' : 'BeefAPI connected',
        description: zh ? 'GPT Pro 与默认 coding 模型已就绪。' : 'GPT Pro and the default coding model are ready.',
      }
    case 'offline':
      if (!hasBeefApiAccountMetadata(state)) {
        return {
          kicker: zh ? '登录服务暂不可用' : 'Sign-in service unavailable',
          title: zh ? '暂时无法连接 BeefAPI' : 'Could not reach BeefAPI',
          description: zh ? '尚未建立本机账号连接。恢复网络后重试。' : 'No local account connection was created. Reconnect after the network recovers.',
        }
      }
      return {
        kicker: zh ? '连接已中断' : 'Connection interrupted',
        title: zh ? 'BeefAPI 暂时离线' : 'BeefAPI is temporarily offline',
        description: zh ? '本机 credential 仍安全保留。恢复网络后重试连接。' : 'Your local credential remains secure. Reconnect after the network recovers.',
      }
    case 'denied':
      return {
        kicker: zh ? '授权未完成' : 'Authorization not completed',
        title: zh ? '本次登录已取消' : 'This sign-in was cancelled',
        description: zh ? '没有 credential 写入本机，可以重新开始登录。' : 'No credential was stored. You can start again.',
      }
    case 'expired':
      return {
        kicker: zh ? '授权已过期' : 'Authorization expired',
        title: zh ? '验证码已过期' : 'The verification code expired',
        description: zh ? '重新登录会生成新的浏览器确认会话。' : 'Sign in again to create a new browser confirmation.',
      }
    case 'entitlement_missing':
      return {
        kicker: zh ? '账号不满足要求' : 'Account requirement missing',
        title: zh ? '当前账号没有 GPT Pro 权限' : 'This account does not have GPT Pro',
        description: zh ? '请在 BeefAPI 检查账号分组后再重新登录。' : 'Check your BeefAPI account group before signing in again.',
      }
    case 'credential_store_failed':
      return {
        kicker: zh ? '本机安全存储不可用' : 'Secure storage unavailable',
        title: zh ? '无法保存登录凭据' : 'Could not save the sign-in credential',
        description: zh ? 'Beefex 未保存凭据，请检查应用数据目录权限后重试登录。' : 'Beefex did not save the credential. Check the app-data directory permissions, then try signing in again.',
      }
    case 'cleanup_required':
      return {
        kicker: zh ? '需要人工清理' : 'Manual cleanup required',
        title: zh ? '请撤销未完成的桌面授权' : 'Revoke the incomplete desktop authorization',
        description: zh ? '打开 BeefAPI Token 管理，撤销对应的 Beefex desktop token 后再重试。' : 'Open BeefAPI token management and revoke the matching Beefex desktop token before retrying.',
      }
  }
}

function primaryActionLabel(action: BeefApiAccountAction, lang: Lang) {
  const zh = lang === 'zh'
  const labels: Record<BeefApiAccountAction, string> = {
    none: zh ? '请稍候' : 'Please wait',
    start: zh ? '登录 BeefAPI' : 'Sign in to BeefAPI',
    reopen: zh ? '重新打开浏览器' : 'Reopen browser',
    continue: zh ? '进入 Beefex' : 'Enter Beefex',
    retry: zh ? '重新登录' : 'Sign in again',
    reconnect: zh ? '重试连接' : 'Reconnect',
    manage_account: zh ? '打开 BeefAPI 账号' : 'Open BeefAPI account',
    manage_tokens: zh ? '管理 BeefAPI Token' : 'Manage BeefAPI tokens',
  }
  return labels[action]
}

function formatGroup(group: string | undefined, lang: Lang) {
  if (!group) return lang === 'zh' ? '未返回' : 'Not reported'
  return group.toLowerCase() === 'gpt-pro' ? 'GPT Pro' : group
}

function formatModel(model: string | undefined, lang: Lang) {
  return model ? model.toUpperCase() : lang === 'zh' ? '未返回' : 'Not reported'
}
