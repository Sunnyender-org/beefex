import { Download, ExternalLink, RefreshCw } from 'lucide-react'
import { Button } from '../components/Button'
import type { InAppDownloadState } from './inAppUpdate'

export function InAppUpdateActions({
  state,
  percent,
  error,
  downloadAndInstallLabel,
  downloadingLabel,
  retryLabel,
  openDownloadPageLabel,
  laterLabel,
  onDownloadAndInstall,
  onOpenReleasePage,
  onLater,
}: {
  state: InAppDownloadState
  percent: number
  error: string
  downloadAndInstallLabel: string
  downloadingLabel: string
  retryLabel: string
  openDownloadPageLabel: string
  laterLabel: string
  onDownloadAndInstall: () => void
  onOpenReleasePage: () => void
  onLater: () => void
}) {
  return (
    <>
      {state === 'downloading' && (
        <div className="mb-3">
          <div className="mb-1 flex items-center justify-between kv-panel-body">
            <span>{downloadingLabel}</span>
            <span className="font-mono tabular-nums">{percent}%</span>
          </div>
          <div className="kv-progress">
            <div style={{ width: `${percent}%` }} />
          </div>
        </div>
      )}

      {state === 'failed' && error && (
        <div className="kv-inline-error mb-3">
          {error}
        </div>
      )}

      <div className="flex flex-wrap gap-2">
        {state === 'idle' && (
          <>
            <Button
              variant="primary"
              onClick={onDownloadAndInstall}
              data-tauri-drag-region="false"
            >
              <Download size={12} />
              {downloadAndInstallLabel}
            </Button>
            <Button
              onClick={onOpenReleasePage}
              data-tauri-drag-region="false"
            >
              <ExternalLink size={12} />
              {openDownloadPageLabel}
            </Button>
          </>
        )}
        {state === 'downloading' && (
          <Button disabled>
            <RefreshCw size={12} className="animate-spin" />
            {downloadingLabel}
          </Button>
        )}
        {state === 'failed' && (
          <>
            <Button
              variant="primary"
              onClick={onDownloadAndInstall}
              data-tauri-drag-region="false"
            >
              <RefreshCw size={12} />
              {retryLabel}
            </Button>
            <Button
              onClick={onOpenReleasePage}
              data-tauri-drag-region="false"
            >
              <ExternalLink size={12} />
              {openDownloadPageLabel}
            </Button>
          </>
        )}
        <Button
          variant="ghost"
          onClick={onLater}
          data-tauri-drag-region="false"
        >
          {laterLabel}
        </Button>
      </div>
    </>
  )
}
