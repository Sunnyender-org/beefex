const ACTION_TITLE = "__BEEFEX_MANAGED_CLIENTS_APPLY__"

type ToolContext = { ui: { input: (title: string, placeholder?: string) => Promise<string | undefined> } }
type ExtensionApi = { registerTool: (definition: Record<string, unknown>) => void }

export default function registerBeefexClientSetup(pi: ExtensionApi) {
  pi.registerTool({
    name: "configure_beefapi_clients",
    label: "Configure BeefAPI clients",
    description: "Configure Codex with Image2, Claude Code, Claude Desktop, and Grok from the current Beefex login.",
    promptSnippet: "Configure supported local coding clients to use BeefAPI",
    promptGuidelines: [
      "Use configure_beefapi_clients when the user explicitly asks Beefex to configure Codex, Image2, Claude Code, Claude Desktop, or Grok for BeefAPI.",
    ],
    parameters: {
      type: "object",
      "~kind": "Object",
      properties: {
        codexModel: { type: "string", "~kind": "String", "~optional": true, description: "Optional BeefAPI-allowed Codex model; omit to use the account default." },
      },
      additionalProperties: false,
    },
    async execute(_toolCallId: string, params: { codexModel?: string }, _signal: unknown, _onUpdate: unknown, ctx: ToolContext) {
      const result = await ctx.ui.input(ACTION_TITLE, JSON.stringify({ codexModel: params.codexModel ?? null }))
      if (!result) throw new Error("beefex_client_setup_unavailable")
      const parsed = JSON.parse(result) as { ok: boolean; error?: string; configured?: string[] }
      if (!parsed.ok) throw new Error(parsed.error || "beefex_client_setup_failed")
      return {
        content: [{ type: "text", text: `Configured: ${(parsed.configured ?? []).join(", ")}` }],
        details: parsed,
      }
    },
  })
}
