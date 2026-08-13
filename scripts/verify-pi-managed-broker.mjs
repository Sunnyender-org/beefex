import { spawn } from 'node:child_process'
import { createServer } from 'node:http'
import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { dirname, join, resolve } from 'node:path'
import { createInterface } from 'node:readline'
import { fileURLToPath } from 'node:url'

const repo = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const pi = join(repo, 'node_modules', '.bin', 'pi')
const extension = join(repo, 'src-tauri', 'resources', 'pi', 'beefex-managed-provider-extension.ts')
const capability = '0123456789abcdef0123456789abcdef'
const requestPath = `/${capability}/v1/responses`

const root = await mkdtemp(join(tmpdir(), 'beefex-pi-managed-broker-'))
const observedRequests = []
const server = createServer((request, response) => {
  const chunks = []
  request.on('data', (chunk) => chunks.push(chunk))
  request.on('end', () => {
    observedRequests.push({
      method: request.method,
      url: request.url,
      authorization: request.headers.authorization,
      body: Buffer.concat(chunks).toString('utf8'),
    })
    response.writeHead(200, {
      'content-type': 'text/event-stream',
      'cache-control': 'no-store',
    })
    response.write('data: {"type":"response.output_item.added","output_index":0,"item":{"id":"msg_fixture","type":"message","role":"assistant","status":"in_progress","content":[]}}\n\n')
    response.write('data: {"type":"response.output_text.delta","output_index":0,"content_index":0,"delta":"BEEFEX_PI_MANAGED_BROKER_OK"}\n\n')
    response.write('data: {"type":"response.output_item.done","output_index":0,"item":{"id":"msg_fixture","type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":"BEEFEX_PI_MANAGED_BROKER_OK","annotations":[]}]}}\n\n')
    response.write('data: {"type":"response.completed","response":{"status":"completed","usage":{"input_tokens":3,"output_tokens":1,"total_tokens":4}}}\n\n')
    response.end()
  })
})

await new Promise((resolveListen, rejectListen) => {
  server.once('error', rejectListen)
  server.listen(0, '127.0.0.1', resolveListen)
})
const address = server.address()
if (!address || typeof address === 'string') throw new Error('broker fixture did not bind TCP')

async function runPi(sessionFile, prompt) {
  const args = [
    '--mode', 'rpc',
    '--no-approve',
    '--model', 'beefex-managed/gpt-5.6-sol',
    '--session-dir', join(root, 'sessions'),
    '--extension', extension,
  ]
  if (sessionFile) args.push('--session', sessionFile)
  const child = spawn(pi, args, {
    cwd: root,
    env: {
      PATH: `${dirname(process.execPath)}:${join(repo, 'node_modules', '.bin')}:/usr/bin:/bin`,
      TMPDIR: tmpdir(),
      PI_CODING_AGENT_DIR: join(root, 'agent'),
      PI_CODING_AGENT_SESSION_DIR: join(root, 'sessions'),
      PI_SKIP_VERSION_CHECK: '1',
      PI_TELEMETRY: '0',
      BEEFEX_PI_BROKER_URL: `http://127.0.0.1:${address.port}/${capability}/v1`,
      BEEFEX_PI_MODEL: 'gpt-5.6-sol',
    },
    stdio: ['pipe', 'pipe', 'pipe'],
  })
  let stdout = ''
  let stderr = ''
  let promptCompleted = false
  let sessionState
  child.stderr.setEncoding('utf8')
  child.stderr.on('data', (chunk) => { stderr += chunk })
  const lines = createInterface({ input: child.stdout })
  const finished = new Promise((resolveFinished, rejectFinished) => {
    const timer = setTimeout(() => {
      child.kill('SIGKILL')
      rejectFinished(new Error(`managed broker fixture timeout: ${stderr}`))
    }, 20_000)
    child.once('error', rejectFinished)
    child.once('exit', (code) => {
      clearTimeout(timer)
      if (code === 0 && promptCompleted && sessionState) resolveFinished(undefined)
      else rejectFinished(new Error(`managed broker fixture exited code=${code}: ${stderr}`))
    })
  })
  lines.on('line', (line) => {
    stdout += `${line}\n`
    let event
    try { event = JSON.parse(line) } catch { return }
    if (event.type === 'response' && event.command === 'prompt' && event.success !== true) {
      child.stdin.end()
    }
    if (event.type === 'agent_end') {
      promptCompleted = true
      child.stdin.write(`${JSON.stringify({ id: 'managed-broker-state', type: 'get_state' })}\n`)
    }
    if (event.type === 'response' && event.command === 'get_state' && event.success) {
      sessionState = event.data
      child.stdin.end()
    }
  })
  child.stdin.write(`${JSON.stringify({ id: 'managed-broker-prompt', type: 'prompt', message: prompt })}\n`)
  await finished
  return { stdout, stderr, sessionState }
}

try {
  const first = await runPi(undefined, 'Reply with the fixture marker.')
  const resumed = await runPi(first.sessionState.sessionFile, 'Continue the same managed session.')
  if (observedRequests.length !== 2) {
    throw new Error(`Pi reached the managed broker ${observedRequests.length} times instead of twice`)
  }
  for (const observedRequest of observedRequests) {
    if (observedRequest.method !== 'POST' || observedRequest.url !== requestPath) {
      throw new Error(`unexpected managed broker route: ${observedRequest.method} ${observedRequest.url}`)
    }
    if (observedRequest.authorization !== 'Bearer beefex-parent-broker') {
      throw new Error('Pi did not use the parent-broker capability header')
    }
    const body = JSON.parse(observedRequest.body)
    if (body.model !== 'gpt-5.6-sol' || body.stream !== true) {
      throw new Error(`unexpected managed request body: ${observedRequest.body}`)
    }
  }
  if (!first.stdout.includes('BEEFEX_PI_MANAGED_BROKER_OK') || !resumed.stdout.includes('BEEFEX_PI_MANAGED_BROKER_OK')) {
    throw new Error('Pi did not stream the managed broker response before and after resume')
  }
  if (first.sessionState.sessionId !== resumed.sessionState.sessionId) {
    throw new Error('fresh Pi process did not resume the same managed session id')
  }
  const resumedBody = JSON.parse(observedRequests[1].body)
  if (!JSON.stringify(resumedBody.input).includes('Reply with the fixture marker.')) {
    throw new Error('resumed managed request did not retain prior session context')
  }
  process.stdout.write(`${JSON.stringify({ receipt: 'BEEFEX_PI_MANAGED_BROKER_OK', model: resumedBody.model, route: requestPath, sessionId: resumed.sessionState.sessionId, processRestarts: 1 })}\n`)
} finally {
  server.close()
  await rm(root, { recursive: true, force: true })
}
