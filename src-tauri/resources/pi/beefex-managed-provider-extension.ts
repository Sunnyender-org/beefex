declare const process: {
  env: Record<string, string | undefined>
}

type ExtensionApi = {
  registerProvider: (name: string, config: Record<string, unknown>) => void
}

function takeEnvironment(name: string): string {
  const value = process.env[name]?.trim() ?? ""
  delete process.env[name]
  if (!value) throw new Error(`${name.toLowerCase()}_missing`)
  return value
}

function trustedLoopbackEndpoint(raw: string): string {
  const endpoint = new URL(raw)
  if (endpoint.protocol !== "http:" || endpoint.hostname !== "127.0.0.1") {
    throw new Error("beefex_pi_broker_untrusted")
  }
  if (!/^\/[a-f0-9]{32}\/v1$/.test(endpoint.pathname) || endpoint.search || endpoint.hash) {
    throw new Error("beefex_pi_broker_invalid_capability")
  }
  return endpoint.toString().replace(/\/$/, "")
}

export default function registerManagedProvider(pi: ExtensionApi) {
  const rawBaseUrl = takeEnvironment("BEEFEX_PI_BROKER_URL")
  const model = takeEnvironment("BEEFEX_PI_MODEL")
  const baseUrl = trustedLoopbackEndpoint(rawBaseUrl)
  if (!/^[A-Za-z0-9._-]{1,128}$/.test(model)) {
    throw new Error("beefex_pi_model_invalid")
  }
  pi.registerProvider("beefex-managed", {
    name: "BeefAPI",
    baseUrl,
    apiKey: "beefex-parent-broker",
    api: "openai-responses",
    models: [{
      id: model,
      name: model,
      reasoning: true,
      input: ["text", "image"],
      cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
      contextWindow: 200000,
      maxTokens: 32768,
    }],
  })
}
