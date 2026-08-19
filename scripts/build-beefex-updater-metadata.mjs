#!/usr/bin/env node
import { readFileSync } from 'node:fs'
import { pathToFileURL } from 'node:url'

const ALPHA_VERSION = /^0\.1\.0-alpha\.\d+$/
const SHA_LINE = /^([0-9a-fA-F]{64})[ \t]+\*?(\S+)$/

export function normalizeAlphaVersion(version) {
  const trimmed = String(version ?? '').trim()
  const normalized = trimmed.startsWith('v') ? trimmed.slice(1) : trimmed
  if (!ALPHA_VERSION.test(normalized)) {
    throw new Error('version must match 0.1.0-alpha.N')
  }
  return normalized
}

export function parseSha256sums(text) {
  const checksums = {}
  for (const raw of String(text).split(/\r?\n/)) {
    const line = raw.trim()
    if (!line || line.startsWith('#')) continue
    const match = line.match(SHA_LINE)
    if (!match) {
      throw new Error(`malformed SHA line: ${raw}`)
    }
    checksums[match[2]] = match[1].toLowerCase()
  }
  return checksums
}

export function buildUpdaterDocument({ version, commit = '', checksums }) {
  const normalized = normalizeAlphaVersion(version)
  const mac = checksums['beefex-desktop-mac-arm64.dmg']
  const win = checksums['beefex-desktop-win-x64.exe']
  if (!mac || !win) {
    throw new Error('SHA256SUMS.txt must include both current Alpha installer names')
  }
  const base = 'https://pub-e540a6ea6d6e4af19d7f5fc4d1f07c47.r2.dev/beefex/releases/latest'
  return {
    schema_version: 'beefex.updater.v1',
    product: 'Beefex',
    identifier: 'com.beefapi.beefex',
    version: normalized,
    tag: `v${normalized}`,
    source_commit: commit,
    channel: 'alpha',
    notes: [`Beefex ${normalized}`],
    assets: {
      'macos-aarch64': {
        file: 'beefex-desktop-mac-arm64.dmg',
        url: `${base}/beefex-desktop-mac-arm64.dmg`,
        sha256: mac,
      },
      'windows-x86_64': {
        file: 'beefex-desktop-win-x64.exe',
        url: `${base}/beefex-desktop-win-x64.exe`,
        sha256: win,
      },
    },
  }
}

function take(args, name) {
  const index = args.indexOf(name)
  if (index < 0) return null
  return args[index + 1] ?? null
}

const isMain = Boolean(process.argv[1]) && pathToFileURL(process.argv[1]).href === import.meta.url
if (isMain) {
  const args = process.argv.slice(2)
  const version = take(args, '--version') || '0.1.0-alpha.6'
  const commit = take(args, '--commit') || ''
  const sumsPath = take(args, '--sha256sums')
  if (!sumsPath) {
    throw new Error('usage: build-beefex-updater-metadata.mjs --sha256sums SHA256SUMS.txt [--version 0.1.0-alpha.6] [--commit <release-commit>]')
  }
  const checksums = parseSha256sums(readFileSync(sumsPath, 'utf8'))
  process.stdout.write(`${JSON.stringify(buildUpdaterDocument({ version, commit, checksums }), null, 2)}\n`)
}
