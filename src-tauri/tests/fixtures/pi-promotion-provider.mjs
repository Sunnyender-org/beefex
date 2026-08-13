import { createAssistantMessageEventStream } from "@earendil-works/pi-ai"

function emptyUsage() {
  return {
    input: 1,
    output: 1,
    cacheRead: 0,
    cacheWrite: 0,
    totalTokens: 2,
    cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
  }
}

function streamPromotionFixture(model, context) {
  const stream = createAssistantMessageEventStream()
  queueMicrotask(() => {
    const toolResults = context.messages.filter((message) => message.role === "toolResult")
    const output = {
      role: "assistant",
      content: [],
      api: model.api,
      provider: model.provider,
      model: model.id,
      usage: emptyUsage(),
      stopReason: "pending",
      timestamp: Date.now(),
    }
    stream.push({ type: "start", partial: output })

    if (toolResults.length === 0) {
      const toolCall = {
        type: "toolCall",
        id: "fixture-edit-call",
        name: "edit",
        arguments: { path: "promotion.txt", oldText: "before\n", newText: "created-by-pi\n" },
      }
      output.content.push(toolCall)
      stream.push({ type: "toolcall_start", contentIndex: 0, partial: output })
      stream.push({ type: "toolcall_delta", contentIndex: 0, delta: JSON.stringify(toolCall.arguments), partial: output })
      stream.push({ type: "toolcall_end", contentIndex: 0, toolCall, partial: output })
      output.stopReason = "toolUse"
    } else if (toolResults.length === 1) {
      const toolCall = {
        type: "toolCall",
        id: "fixture-bash-call",
        name: "bash",
        arguments: { command: "wc -c promotion.txt" },
      }
      output.content.push(toolCall)
      stream.push({ type: "toolcall_start", contentIndex: 0, partial: output })
      stream.push({ type: "toolcall_delta", contentIndex: 0, delta: JSON.stringify(toolCall.arguments), partial: output })
      stream.push({ type: "toolcall_end", contentIndex: 0, toolCall, partial: output })
      output.stopReason = "toolUse"
    } else {
      const text = "Promotion fixture completed with one file mutation and one shell command."
      output.content.push({ type: "text", text })
      stream.push({ type: "text_start", contentIndex: 0, partial: output })
      stream.push({ type: "text_delta", contentIndex: 0, delta: text, partial: output })
      stream.push({ type: "text_end", contentIndex: 0, content: text, partial: output })
      output.stopReason = "stop"
    }

    stream.push({ type: "done", reason: output.stopReason, message: output })
    stream.end()
  })
  return stream
}

export default function registerPromotionProvider(pi) {
  pi.registerProvider("beefex-fixture", {
    baseUrl: "http://127.0.0.1/unused",
    apiKey: "local-fixture-only",
    api: "openai-responses",
    models: [{
      id: "promotion",
      name: "Beefex promotion fixture",
      reasoning: false,
      input: ["text"],
      cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
      contextWindow: 8192,
      maxTokens: 1024,
    }],
    streamSimple: streamPromotionFixture,
  })
}
