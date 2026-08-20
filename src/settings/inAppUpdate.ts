export type InAppDownloadState = 'idle' | 'downloading' | 'failed'

export type DownloadAndInstallResult =
  | { ok: true; path: string }
  | { ok: false; phase: 'download' | 'install'; error: string }

export function formatInAppUpdateError(error: unknown): string {
  if (typeof error === 'string' && error.trim()) return error
  if (error instanceof Error && error.message.trim()) return error.message
  return String(error)
}

export function inAppUpdateFailureText(
  phase: 'download' | 'install',
  error: string,
  labels: { downloadFailed: string; installFailed: string },
): string {
  const prefix = phase === 'install' ? labels.installFailed : labels.downloadFailed
  return `${prefix}: ${error}`
}

/** One user action: download the verified package, then run the existing install command. */
export async function downloadAndInstallUpdate(input: {
  version: string
  sha256?: string
  downloadUpdate: (version: string, sha256?: string) => Promise<string>
  installUpdate: (path: string, version?: string) => Promise<void>
}): Promise<DownloadAndInstallResult> {
  let path: string
  try {
    path = await input.downloadUpdate(input.version, input.sha256)
  } catch (error) {
    return { ok: false, phase: 'download', error: formatInAppUpdateError(error) }
  }
  try {
    await input.installUpdate(path, input.version)
  } catch (error) {
    return { ok: false, phase: 'install', error: formatInAppUpdateError(error) }
  }
  return { ok: true, path }
}
