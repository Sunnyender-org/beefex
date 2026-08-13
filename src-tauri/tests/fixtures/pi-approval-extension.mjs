import { writeFile } from "node:fs/promises"

export default function registerApprovalFixture(pi) {
  pi.registerCommand("beefex-approval-fixture", {
    description: "Exercise the Pi RPC approval bridge without a model account",
    handler: async (args, ctx) => {
      const target = args.trim()
      const approved = await ctx.ui.confirm(
        "Beefex approval fixture",
        JSON.stringify({ toolCallId: "fixture-write", toolName: "write", path: target }),
      )
      if (approved) await writeFile(target, "approved-by-pi-rpc\n", "utf8")
    },
  })
}
