import { createHash } from 'node:crypto'
import { cpSync, existsSync, mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs'
import { dirname, join, relative, resolve } from 'node:path'
import { execFileSync } from 'node:child_process'

const repositoryRoot = resolve(dirname(new URL(import.meta.url).pathname), '..')
const vendorRoot = join(repositoryRoot, 'src', 'bflabs', 'vendor')
const manifestPath = join(vendorRoot, 'MANIFEST.json')
const upstream = {
  repository: 'https://github.com/Sunnyender-org/bflabs-ui',
  commit: '54997785cc4076a5e23f4cd31732065945442cad',
  packageRoot: 'packages/ui',
}

function sha256(path) {
  return createHash('sha256').update(readFileSync(path)).digest('hex')
}

function listFiles(root, current = root) {
  return readdirSync(current, { withFileTypes: true })
    .flatMap((entry) => {
      const path = join(current, entry.name)
      return entry.isDirectory() ? listFiles(root, path) : [relative(root, path).replaceAll('\\', '/')]
    })
    .sort()
}

function writeManifest() {
  const files = listFiles(vendorRoot).filter((path) => path !== 'MANIFEST.json')
  const manifest = {
    schemaVersion: 1,
    upstream,
    files: Object.fromEntries(files.map((path) => [path, sha256(join(vendorRoot, path))])),
  }
  writeFileSync(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`)
}

function verifyManifest() {
  if (!existsSync(manifestPath)) throw new Error('missing BFLabs vendor manifest')
  const manifest = JSON.parse(readFileSync(manifestPath, 'utf8'))
  if (manifest.upstream?.commit !== upstream.commit) throw new Error('unexpected BFLabs upstream commit')
  const actualFiles = listFiles(vendorRoot).filter((path) => path !== 'MANIFEST.json')
  const expectedFiles = Object.keys(manifest.files).sort()
  if (JSON.stringify(actualFiles) !== JSON.stringify(expectedFiles)) throw new Error('BFLabs vendor file set drifted')
  for (const path of expectedFiles) {
    if (sha256(join(vendorRoot, path)) !== manifest.files[path]) {
      throw new Error(`BFLabs vendor file drifted: ${path}`)
    }
  }
  return manifest
}

function resolveSourceRoot(source) {
  const root = resolve(source)
  const head = execFileSync('git', ['-C', root, 'rev-parse', 'HEAD'], { encoding: 'utf8' }).trim()
  if (head !== upstream.commit) throw new Error(`BFLabs source HEAD ${head} does not match ${upstream.commit}`)
  return join(root, upstream.packageRoot)
}

function sync(source) {
  const packageRoot = resolveSourceRoot(source)
  rmSync(vendorRoot, { recursive: true, force: true })
  mkdirSync(vendorRoot, { recursive: true })
  cpSync(join(packageRoot, 'src'), join(vendorRoot, 'src'), { recursive: true })
  cpSync(join(packageRoot, 'LICENSE'), join(vendorRoot, 'LICENSE'))
  cpSync(join(packageRoot, 'package.json'), join(vendorRoot, 'package.json'))
  writeFileSync(join(vendorRoot, 'UPSTREAM.json'), `${JSON.stringify(upstream, null, 2)}\n`)
  writeManifest()
}

function verifyUpstream(source, manifest) {
  const packageRoot = resolveSourceRoot(source)
  for (const path of Object.keys(manifest.files)) {
    const sourcePath = path === 'LICENSE' || path === 'package.json'
      ? join(packageRoot, path)
      : path === 'UPSTREAM.json'
        ? null
        : join(packageRoot, path)
    if (sourcePath && sha256(sourcePath) !== manifest.files[path]) {
      throw new Error(`vendored BFLabs file differs from upstream: ${path}`)
    }
  }
}

const args = process.argv.slice(2)
const syncIndex = args.indexOf('--sync')
if (syncIndex >= 0) {
  const source = args[syncIndex + 1]
  if (!source) throw new Error('--sync requires a BFLabs repository path')
  sync(source)
}

const manifest = verifyManifest()
const appCss = readFileSync(join(repositoryRoot, 'src', 'index.css'), 'utf8')
const inputBarSource = readFileSync(join(repositoryRoot, 'src', 'chat', 'InputBar.tsx'), 'utf8')
const chatSource = readFileSync(join(repositoryRoot, 'src', 'chat', 'Chat.tsx'), 'utf8')
const sidebarSource = readFileSync(join(repositoryRoot, 'src', 'chat', 'Sidebar.tsx'), 'utf8')
if (!appCss.includes('@import "./bflabs/vendor/src/styles/index.css";')) {
  throw new Error('Beefex is not importing the canonical BFLabs styles')
}
if (appCss.includes('transparent calc(50% - 1px)')) {
  throw new Error('rejected empty Task center divider returned')
}
if (appCss.includes('border-right: 3px solid var(--beef-active);')) {
  throw new Error('rejected orange user-message edge returned')
}
if (inputBarSource.includes('focus-within:border-[var(--beef-active)]')) {
  throw new Error('rejected orange composer focus border returned')
}
if (!inputBarSource.includes('focus-visible:outline-none')) {
  throw new Error('composer textarea must suppress the canonical orange focus outline')
}
if (!appCss.includes('.chat-composer-shell textarea:focus-visible') || !appCss.includes('outline: none;')) {
  throw new Error('composer textarea focus override must outrank the canonical global outline')
}
if (!inputBarSource.includes('<ManagedModelSelector') || chatSource.includes('<ManagedModelSelector')) {
  throw new Error('managed model selection must live inside the composer, not the titlebar')
}
if (!inputBarSource.includes('models={managedModels}') || !chatSource.includes('managedModels={managedAllowedModels}')) {
  throw new Error('composer must continue passing only the server-allowed model catalog')
}
if (!inputBarSource.includes('<ManagedModelSelector')) {
  throw new Error('managed model selector must remain inside the composer')
}
const managedModelSource = readFileSync(join(repositoryRoot, 'src', 'chat', 'ManagedModelSelector.tsx'), 'utf8')
if (!managedModelSource.includes('beef-managed-model-popover') || managedModelSource.includes('bg-[var(--beef-surface)]')) {
  throw new Error('managed model popover must use its opaque overlay surface, not the translucent card surface')
}
if (!appCss.includes('--beef-overlay-surface:') || !appCss.includes('.beef-managed-model-popover')) {
  throw new Error('opaque managed-model overlay token is missing')
}
if (!inputBarSource.includes('max-w-6xl') || !chatSource.includes('max-w-6xl')) {
  throw new Error('composer width convergence regressed')
}
if (!inputBarSource.includes('rows={2}') || !inputBarSource.includes('min-h-[52px]')) {
  throw new Error('composer must expose a two-line input by default')
}
if (!sidebarSource.includes('aria-selected={activeTab === tab}')) {
  throw new Error('sidebar task/project tabs must expose a distinct selected state')
}
if (!chatSource.includes("'Create anything with Beefex' : '在 Beefex 上创造一切'")) {
  throw new Error('empty Task hero must use the owner-confirmed Beefex product headline')
}
if (chatSource.includes('Open a project and describe the change') || chatSource.includes('打开项目，然后描述要修改的内容')) {
  throw new Error('rejected instructional empty Task headline returned')
}
const sourceIndex = args.indexOf('--source')
if (sourceIndex >= 0) {
  const source = args[sourceIndex + 1]
  if (!source) throw new Error('--source requires a BFLabs repository path')
  verifyUpstream(source, manifest)
}

console.log(`BFLabs source verified: ${upstream.commit} (${Object.keys(manifest.files).length} files)`)
