#!/usr/bin/env node

import { spawnSync } from 'node:child_process'
import {
  chmodSync,
  copyFileSync,
  cpSync,
  existsSync,
  mkdirSync,
  readFileSync,
  rmSync,
} from 'node:fs'
import { homedir } from 'node:os'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const scriptDir = dirname(fileURLToPath(import.meta.url))
const repo = resolve(scriptDir, '..')
const piPackage = resolve(repo, 'node_modules/@earendil-works/pi-coding-agent')
const outputDir = resolve(repo, 'src-tauri/resources/pi/bin')
const supportedPlatforms = new Set(['darwin', 'win32'])
const outputBinary = resolve(outputDir, process.platform === 'win32' ? 'pi.exe' : 'pi')

if (!supportedPlatforms.has(process.platform)) {
  console.log(`[build-pi-runtime] skipped: unsupported platform ${process.platform}`)
  process.exit(0)
}

const packageJson = JSON.parse(readFileSync(resolve(piPackage, 'package.json'), 'utf8'))
if (packageJson.version !== '0.84.1') {
  throw new Error(
    `[build-pi-runtime] expected Pi 0.84.1, found ${packageJson.version ?? 'unknown'}`,
  )
}

const userBun = join(homedir(), '.bun/bin', process.platform === 'win32' ? 'bun.exe' : 'bun')
const bun = process.env.BUN_BIN || (existsSync(userBun) ? userBun : 'bun')

rmSync(outputDir, { recursive: true, force: true })
mkdirSync(outputDir, { recursive: true })

const compile = spawnSync(
  bun,
  [
    'build',
    resolve(piPackage, 'dist/bun/cli.js'),
    '--compile',
    '--no-compile-autoload-bunfig',
    '--outfile',
    outputBinary,
  ],
  { cwd: repo, stdio: 'inherit' },
)
if (compile.status !== 0) {
  throw new Error(`[build-pi-runtime] Bun compile failed with status ${compile.status}`)
}

mkdirSync(resolve(outputDir, 'theme'), { recursive: true })
mkdirSync(resolve(outputDir, 'assets'), { recursive: true })
mkdirSync(resolve(outputDir, 'export-html/vendor'), { recursive: true })
cpSync(resolve(piPackage, 'dist/modes/interactive/theme'), resolve(outputDir, 'theme'), {
  recursive: true,
})
cpSync(resolve(piPackage, 'dist/modes/interactive/assets'), resolve(outputDir, 'assets'), {
  recursive: true,
})
copyFileSync(
  resolve(piPackage, 'dist/core/export-html/template.html'),
  resolve(outputDir, 'export-html/template.html'),
)
cpSync(
  resolve(piPackage, 'dist/core/export-html/vendor'),
  resolve(outputDir, 'export-html/vendor'),
  { recursive: true },
)
for (const name of ['package.json', 'README.md', 'CHANGELOG.md']) {
  copyFileSync(resolve(piPackage, name), resolve(outputDir, name))
}
copyFileSync(
  resolve(piPackage, 'node_modules/@silvia-odwyer/photon-node/photon_rs_bg.wasm'),
  resolve(outputDir, 'photon_rs_bg.wasm'),
)
if (process.platform !== 'win32') {
  chmodSync(outputBinary, 0o755)
}

const version = spawnSync(outputBinary, ['--version'], { encoding: 'utf8' })
if (version.status !== 0 || version.stdout.trim() !== '0.84.1') {
  throw new Error(
    `[build-pi-runtime] compiled runtime readback failed: ${version.stdout || version.stderr}`,
  )
}

console.log(`[build-pi-runtime] Pi ${version.stdout.trim()} -> ${outputBinary}`)
