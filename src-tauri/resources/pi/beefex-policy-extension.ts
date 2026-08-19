// eslint-disable-next-line @typescript-eslint/ban-ts-comment -- Node types differ across package hosts.
// @ts-ignore Pi loads this resource in Node; the renderer graph may omit Node types.
import { existsSync, realpathSync } from "node:fs"
// eslint-disable-next-line @typescript-eslint/ban-ts-comment -- Node types differ across package hosts.
// @ts-ignore Pi loads this resource in Node; the renderer graph may omit Node types.
import { dirname, isAbsolute, relative, resolve } from "node:path"

declare const process: { cwd: () => string }

function nearestExisting(path: string): string {
  let cursor = path
  while (!existsSync(cursor)) {
    const parent = dirname(cursor)
    if (parent === cursor) break
    cursor = parent
  }
  return realpathSync(cursor)
}

function resolvesInside(root: string, candidate: string): boolean {
  const target = resolve(root, candidate)
  const existing = nearestExisting(target)
  const suffix = relative(existing, target)
  const resolved = resolve(existing, suffix)
  const fromRoot = relative(root, resolved)
  return fromRoot === "" || (!fromRoot.startsWith("..") && !isAbsolute(fromRoot))
}

function requestedPath(input: Record<string, unknown>): string | undefined {
  const value = input.path
  return typeof value === "string" && value.trim() ? value : undefined
}

function shellLooksProjectScoped(command: string): boolean {
  if (!command.trim()) return false
  if (/[;&|><`$~\\\n\r]/.test(command)) return false
  return command
    .split(/\s+/)
    .filter(Boolean)
    .every((token) => !isAbsolute(token) && token !== ".." && !token.startsWith("../") && !token.includes("/../"))
}

type ToolCallEvent = {
  toolCallId: string
  toolName: string
  input?: Record<string, unknown>
}

type ToolCallDecision = { block: true; reason: string } | undefined

type ExtensionContext = {
  hasUI: boolean
  ui: { confirm: (title: string, message: string) => Promise<boolean> }
}

type ExtensionApi = {
  on: (
    eventName: "tool_call",
    handler: (event: ToolCallEvent, context: ExtensionContext) => Promise<ToolCallDecision>,
  ) => void
}

export default function beefexPolicyExtension(pi: ExtensionApi) {
  const projectRoot = realpathSync(process.cwd())

  pi.on("tool_call", async (event, ctx) => {
    const input = (event.input ?? {}) as Record<string, unknown>
    const path = requestedPath(input)
    if (path && !resolvesInside(projectRoot, path)) {
      return { block: true, reason: "path_outside_project_root" }
    }

    if (event.toolName === "bash") {
      const command = typeof input.command === "string" ? input.command : ""
      if (!shellLooksProjectScoped(command)) {
        return { block: true, reason: "shell_scope_not_provable" }
      }
    }

    const needsApproval = event.toolName === "bash" || event.toolName === "edit" || event.toolName === "write"
    if (!needsApproval) return undefined
    if (!ctx.hasUI) return { block: true, reason: "approval_ui_unavailable" }

    const approved = await ctx.ui.confirm(
      `Beefex tool approval · ${event.toolCallId}`,
      JSON.stringify({ toolCallId: event.toolCallId, toolName: event.toolName, input, projectRoot }),
    )
    return approved ? undefined : { block: true, reason: "tool_denied_by_user" }
  })
}
