import { spawn } from 'node:child_process'
import { access, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { delimiter, dirname, join, resolve } from 'node:path'
import { createInterface } from 'node:readline'
import { fileURLToPath } from 'node:url'

const repo = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const pi = process.env.BEEFEX_PI_BIN || (process.platform === 'win32'
  ? join(repo, 'src-tauri', 'resources', 'pi', 'bin', 'pi.exe')
  : join(repo, 'node_modules', '.bin', 'pi'))
const policy = join(repo, 'src-tauri', 'resources', 'pi', 'beefex-policy-extension.ts')
const fixture = join(repo, 'src-tauri', 'tests', 'fixtures', 'pi-approval-extension.mjs')
const promotionProvider = join(repo, 'src-tauri', 'tests', 'fixtures', 'pi-promotion-provider.mjs')

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

async function runFixture(approved) {
  const root = await mkdtemp(join(tmpdir(), 'beefex-pi-rpc-'))
  const marker = join(root, 'approved.txt')
  const agentDir = join(root, 'agent')
  const sessionDir = join(root, 'sessions')
  const child = spawn(
    pi,
    [
      '--mode', 'rpc',
      '--no-approve',
      '--name', approved ? 'Beefex approval allow' : 'Beefex approval deny',
      '--session-dir', sessionDir,
      '--extension', policy,
      '--extension', fixture,
    ],
    {
      cwd: repo,
      env: piEnvironment(agentDir, sessionDir),
      stdio: ['pipe', 'pipe', 'pipe'],
    },
  )

  let stderr = ''
  child.stderr.setEncoding('utf8')
  child.stderr.on('data', (chunk) => { stderr += chunk })
  const lines = createInterface({ input: child.stdout })
  let approvalSeen = false
  let sessionState
  const finished = new Promise((resolveFinished, rejectFinished) => {
    const timer = setTimeout(() => {
      child.kill('SIGKILL')
      rejectFinished(new Error(`Pi fixture timeout: ${stderr}`))
    }, 15_000)
    child.once('error', rejectFinished)
    child.once('exit', (code) => {
      clearTimeout(timer)
      if (code === 0 && sessionState) resolveFinished(undefined)
      else rejectFinished(new Error(`Pi fixture exited code=${code}: ${stderr}`))
    })
  })

  lines.on('line', (line) => {
    let event
    try { event = JSON.parse(line) } catch { return }
    if (event.type === 'extension_ui_request' && event.method === 'confirm') {
      approvalSeen = true
      child.stdin.write(`${JSON.stringify({ type: 'extension_ui_response', id: event.id, confirmed: approved })}\n`)
    }
    if (event.type === 'response' && event.command === 'prompt' && event.success) {
      child.stdin.write(`${JSON.stringify({ id: 'fixture-state', type: 'get_state' })}\n`)
    }
    if (event.type === 'response' && event.command === 'get_state' && event.success) {
      sessionState = event.data
      child.stdin.end()
    }
  })

  child.stdin.write(`${JSON.stringify({ id: 'fixture-prompt', type: 'prompt', message: `/beefex-approval-fixture ${marker}` })}\n`)
  await finished
  const markerExists = await access(marker).then(() => true, () => false)
  if (!approvalSeen) throw new Error('Pi did not emit an extension confirmation')
  if (markerExists !== approved) throw new Error(`approval result mismatch: approved=${approved} marker=${markerExists}`)
  if (!sessionState?.sessionFile || !sessionState?.sessionId) throw new Error('Pi session state is incomplete')
  if (approved && await readFile(marker, 'utf8') !== 'approved-by-pi-rpc\n') throw new Error('unexpected marker content')
  await rm(root, { recursive: true, force: true })
  return { approved, sessionId: sessionState.sessionId }
}

async function runPromotionFixture() {
  const root = await mkdtemp(join(tmpdir(), 'beefex-pi-promotion-'))
  const agentDir = join(root, 'agent')
  const sessionDir = join(root, 'sessions')
  await writeFile(join(root, 'promotion.txt'), 'before\n', 'utf8')
  const child = spawn(
    pi,
    [
      '--mode', 'rpc',
      '--no-approve',
      '--model', 'beefex-fixture/promotion',
      '--session-dir', sessionDir,
      '--extension', policy,
      '--extension', promotionProvider,
    ],
    {
      cwd: root,
      env: piEnvironment(agentDir, sessionDir),
      stdio: ['pipe', 'pipe', 'pipe'],
    },
  )

  let stderr = ''
  child.stderr.setEncoding('utf8')
  child.stderr.on('data', (chunk) => { stderr += chunk })
  const lines = createInterface({ input: child.stdout })
  const toolStarts = []
  const toolEnds = []
  const approvals = []
  let sessionState
  const finished = new Promise((resolveFinished, rejectFinished) => {
    const timer = setTimeout(() => {
      child.kill('SIGKILL')
      rejectFinished(new Error(`Pi promotion fixture timeout: ${stderr}`))
    }, 20_000)
    child.once('error', rejectFinished)
    child.once('exit', (code) => {
      clearTimeout(timer)
      if (code === 0 && sessionState) resolveFinished(undefined)
      else rejectFinished(new Error(`Pi promotion fixture exited code=${code}: ${stderr}`))
    })
  })

  lines.on('line', (line) => {
    let event
    try { event = JSON.parse(line) } catch { return }
    if (event.type === 'tool_execution_start') toolStarts.push(event)
    if (event.type === 'tool_execution_end') toolEnds.push(event)
    if (event.type === 'extension_ui_request' && event.method === 'confirm') {
      approvals.push(event)
      child.stdin.write(`${JSON.stringify({ type: 'extension_ui_response', id: event.id, confirmed: true })}\n`)
    }
    if (event.type === 'agent_settled') {
      child.stdin.write(`${JSON.stringify({ id: 'promotion-state', type: 'get_state' })}\n`)
    }
    if (event.type === 'response' && event.command === 'get_state' && event.success) {
      sessionState = event.data
      child.stdin.end()
    }
  })

  child.stdin.write(`${JSON.stringify({ id: 'promotion-prompt', type: 'prompt', message: 'Run the deterministic promotion scenario.' })}\n`)
  await finished
  const fileContent = await readFile(join(root, 'promotion.txt'), 'utf8')
  const names = toolStarts.map((event) => event.toolName)
  if (fileContent !== 'created-by-pi\n') throw new Error('Pi promotion file mutation was not observed')
  if (names.join(',') !== 'edit,bash') throw new Error(`unexpected promotion tools: ${names.join(',')}`)
  if (toolEnds.length !== 2 || toolEnds.some((event) => event.isError)) throw new Error('Pi promotion tool result failed')
  const patch = toolEnds[0]?.result?.details?.patch
  if (typeof patch !== 'string' || !patch.includes('-before') || !patch.includes('+created-by-pi')) {
    throw new Error('Pi promotion did not return an observed non-empty patch')
  }
  if (approvals.length !== 2) throw new Error(`expected 2 scoped approvals, observed ${approvals.length}`)
  if (!sessionState?.sessionFile || !sessionState?.sessionId) throw new Error('Pi promotion session state is incomplete')
  const resumed = await resumePromotionFixture(root, agentDir, sessionDir, sessionState)
  await rm(root, { recursive: true, force: true })
  return {
    sessionId: sessionState.sessionId,
    tools: names,
    approvals: approvals.length,
    fileContent,
    patch,
    resumed,
  }
}

async function resumePromotionFixture(root, agentDir, sessionDir, previousState) {
  const child = spawn(
    pi,
    [
      '--mode', 'rpc',
      '--no-approve',
      '--model', 'beefex-fixture/promotion',
      '--session', previousState.sessionFile,
      '--session-dir', sessionDir,
      '--extension', policy,
      '--extension', promotionProvider,
    ],
    {
      cwd: root,
      env: piEnvironment(agentDir, sessionDir),
      stdio: ['pipe', 'pipe', 'pipe'],
    },
  )

  let stderr = ''
  let text = ''
  let toolCalls = 0
  let sessionState
  child.stderr.setEncoding('utf8')
  child.stderr.on('data', (chunk) => { stderr += chunk })
  const lines = createInterface({ input: child.stdout })
  const finished = new Promise((resolveFinished, rejectFinished) => {
    const timer = setTimeout(() => {
      child.kill('SIGKILL')
      rejectFinished(new Error(`Pi resume fixture timeout: ${stderr}`))
    }, 20_000)
    child.once('error', rejectFinished)
    child.once('exit', (code) => {
      clearTimeout(timer)
      if (code === 0 && sessionState) resolveFinished(undefined)
      else rejectFinished(new Error(`Pi resume fixture exited code=${code}: ${stderr}`))
    })
  })

  lines.on('line', (line) => {
    let event
    try { event = JSON.parse(line) } catch { return }
    if (event.type === 'tool_execution_start') toolCalls += 1
    if (event.type === 'message_update' && event.assistantMessageEvent?.type === 'text_delta') {
      text += event.assistantMessageEvent.delta ?? ''
    }
    if (event.type === 'agent_settled') {
      child.stdin.write(`${JSON.stringify({ id: 'resume-state', type: 'get_state' })}\n`)
    }
    if (event.type === 'response' && event.command === 'get_state' && event.success) {
      sessionState = event.data
      child.stdin.end()
    }
  })

  child.stdin.write(`${JSON.stringify({ id: 'resume-prompt', type: 'prompt', message: 'Confirm the prior run is resumable.' })}\n`)
  await finished
  if (sessionState.sessionId !== previousState.sessionId) throw new Error('Pi resume changed the native session id')
  if (toolCalls !== 0) throw new Error(`Pi resume unexpectedly repeated ${toolCalls} tool calls`)
  if (!text.includes('Promotion fixture completed')) throw new Error('Pi resume did not stream a completion')
  return { sessionId: sessionState.sessionId, toolCalls, text }
}

const denied = await runFixture(false)
const allowed = await runFixture(true)
const promotion = await runPromotionFixture()
console.log(JSON.stringify({ receipt: 'BEEFEX_PI_RPC_LOCAL_OK', denied, allowed, promotion }))
