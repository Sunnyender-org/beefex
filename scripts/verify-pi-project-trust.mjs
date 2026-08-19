import { spawn } from 'node:child_process'
import { chmod, mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { delimiter, dirname, join, resolve } from 'node:path'
import { createInterface } from 'node:readline'
import { fileURLToPath } from 'node:url'
import { ProjectTrustStore } from '@earendil-works/pi-coding-agent'

const repo = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const pi = process.env.BEEFEX_PI_BIN || join(
  repo,
  'src-tauri',
  'resources',
  'pi',
  'bin',
  process.platform === 'win32' ? 'pi.exe' : 'pi',
)
const skillName = 'beefex-project-trust-probe'

function piEnvironment(agentDir, sessionDir) {
  const systemRoot = process.env.SystemRoot || process.env.WINDIR || 'C:\\Windows'
  const env = {
    PATH: [
      dirname(process.execPath),
      join(repo, 'node_modules', '.bin'),
      ...(process.platform === 'win32'
        ? [join(systemRoot, 'System32'), systemRoot]
        : ['/usr/bin', '/bin']),
    ].join(delimiter),
    PI_CODING_AGENT_DIR: agentDir,
    PI_CODING_AGENT_SESSION_DIR: sessionDir,
    PI_SKIP_VERSION_CHECK: '1',
    PI_TELEMETRY: '0',
  }
  if (process.platform === 'win32') {
    Object.assign(env, {
      SystemRoot: systemRoot,
      WINDIR: systemRoot,
      ComSpec: process.env.ComSpec || join(systemRoot, 'System32', 'cmd.exe'),
      PATHEXT: process.env.PATHEXT || '.COM;.EXE;.BAT;.CMD',
      TEMP: tmpdir(),
      TMP: tmpdir(),
    })
    for (const key of ['ProgramFiles', 'ProgramFiles(x86)', 'ProgramW6432']) {
      if (process.env[key]) env[key] = process.env[key]
    }
  } else {
    env.TMPDIR = tmpdir()
  }
  return env
}

async function getCommands(projectDir, agentDir, sessionDir) {
  const child = spawn(pi, ['--mode', 'rpc', '--session-dir', sessionDir], {
    cwd: projectDir,
    env: piEnvironment(agentDir, sessionDir),
    stdio: ['pipe', 'pipe', 'pipe'],
  })
  let stderr = ''
  child.stderr.setEncoding('utf8')
  child.stderr.on('data', (chunk) => { stderr += chunk })
  const lines = createInterface({ input: child.stdout })

  return await new Promise((resolveCommands, rejectCommands) => {
    let commands
    const timer = setTimeout(() => {
      child.kill('SIGKILL')
      rejectCommands(new Error(`Pi project trust probe timed out: ${stderr}`))
    }, 15_000)
    child.once('error', rejectCommands)
    child.once('exit', (code) => {
      clearTimeout(timer)
      if (code === 0 && commands) resolveCommands(commands)
      else rejectCommands(new Error(`Pi project trust probe exited code=${code}: ${stderr}`))
    })
    lines.on('line', (line) => {
      let event
      try { event = JSON.parse(line) } catch { return }
      if (event.type !== 'response' || event.command !== 'get_commands') return
      commands = event.data?.commands ?? []
      child.stdin.end()
    })
    child.stdin.write(`${JSON.stringify({ id: 'commands', type: 'get_commands' })}\n`)
  })
}

const root = await mkdtemp(join(tmpdir(), 'beefex-pi-project-trust-'))
const projectDir = join(root, 'project')
const agentDir = join(root, 'agent')
const sessionDir = join(root, 'sessions')
const skillDir = join(projectDir, '.pi', 'skills', skillName)

try {
  await mkdir(skillDir, { recursive: true })
  await mkdir(agentDir, { recursive: true })
  await mkdir(sessionDir, { recursive: true })
  await writeFile(join(skillDir, 'SKILL.md'), `---\nname: ${skillName}\ndescription: Project trust verification probe.\n---\n\nReturn PROJECT_TRUST_PROBE_OK.\n`)

  const before = await getCommands(projectDir, agentDir, sessionDir)
  if (before.some((command) => command.name === `skill:${skillName}`)) {
    throw new Error('untrusted project skill was loaded')
  }

  const trustPath = join(agentDir, 'trust.json')
  new ProjectTrustStore(agentDir).set(projectDir, true)
  await chmod(trustPath, 0o600)
  await chmod(agentDir, 0o700)
  const after = await getCommands(projectDir, agentDir, sessionDir)
  if (!after.some((command) => command.name === `skill:${skillName}`)) {
    throw new Error(`trusted project skill was not discovered by Pi RPC: ${after.map((command) => command.name).join(',')}`)
  }

  console.log(JSON.stringify({
    receipt: 'BEEFEX_PI_PROJECT_TRUST_OK',
    canonicalProject: projectDir,
    untrustedSkillVisible: false,
    trustedSkillVisible: true,
    toolApprovalGranted: false,
  }))
} finally {
  await rm(root, { recursive: true, force: true, maxRetries: 5, retryDelay: 100 })
}
