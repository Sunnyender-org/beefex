import { afterEach, describe, expect, it } from 'vitest'
import registerManagedProvider from '../../src-tauri/resources/pi/beefex-managed-provider-extension'

declare const process: { env: Record<string, string | undefined> }

const BROKER_URL = 'http://127.0.0.1:43123/0123456789abcdef0123456789abcdef/v1'

function clearBrokerEnvironment() {
  delete process.env.BEEFEX_PI_BROKER_URL
  delete process.env.BEEFEX_PI_MODEL
}

afterEach(clearBrokerEnvironment)

describe('Pi managed provider extension', () => {
  it('registers only the loopback parent broker and erases bootstrap capabilities', () => {
    process.env.BEEFEX_PI_BROKER_URL = BROKER_URL
    process.env.BEEFEX_PI_MODEL = 'gpt-5.6-sol'
    const registrations: Array<[string, Record<string, unknown>]> = []

    registerManagedProvider({
      registerProvider(name, config) {
        registrations.push([name, config])
      },
    })

    expect(registrations).toHaveLength(1)
    expect(registrations[0][0]).toBe('beefex-managed')
    expect(registrations[0][1]).toMatchObject({
      baseUrl: BROKER_URL,
      apiKey: 'beefex-parent-broker',
      api: 'openai-responses',
      models: [{ id: 'gpt-5.6-sol' }],
    })
    expect(process.env.BEEFEX_PI_BROKER_URL).toBeUndefined()
    expect(process.env.BEEFEX_PI_MODEL).toBeUndefined()
  })

  it('keeps GPT-family coding models on Responses and routes other chat models to completions', () => {
    const cases: Array<[string, 'openai-responses' | 'openai-completions']> = [
      ['gpt-5.6-sol', 'openai-responses'],
      ['claude-fable-5', 'openai-completions'],
      ['grok-4.6', 'openai-completions'],
      ['glm-5.2', 'openai-completions'],
    ]

    for (const [model, api] of cases) {
      process.env.BEEFEX_PI_BROKER_URL = BROKER_URL
      process.env.BEEFEX_PI_MODEL = model
      const registrations: Array<[string, Record<string, unknown>]> = []
      registerManagedProvider({
        registerProvider(name, config) {
          registrations.push([name, config])
        },
      })
      expect(registrations).toEqual([
        [
          'beefex-managed',
          expect.objectContaining({
            api,
            models: [expect.objectContaining({ id: model })],
          }),
        ],
      ])
    }
  })

  it('rejects non-loopback endpoints after erasing both bootstrap values', () => {
    process.env.BEEFEX_PI_BROKER_URL = 'https://beefapi.com/v1'
    process.env.BEEFEX_PI_MODEL = 'gpt-5.6-sol'

    expect(() => registerManagedProvider({ registerProvider() {} })).toThrow(
      'beefex_pi_broker_untrusted',
    )
    expect(process.env.BEEFEX_PI_BROKER_URL).toBeUndefined()
    expect(process.env.BEEFEX_PI_MODEL).toBeUndefined()
  })

  it('rejects path traversal and invalid model identifiers', () => {
    process.env.BEEFEX_PI_BROKER_URL =
      'http://127.0.0.1:43123/0123456789abcdef0123456789abcdef/../v1'
    process.env.BEEFEX_PI_MODEL = 'gpt-5.6-sol'
    expect(() => registerManagedProvider({ registerProvider() {} })).toThrow(
      'beefex_pi_broker_invalid_capability',
    )

    process.env.BEEFEX_PI_BROKER_URL = BROKER_URL
    process.env.BEEFEX_PI_MODEL = '../secret'
    expect(() => registerManagedProvider({ registerProvider() {} })).toThrow(
      'beefex_pi_model_invalid',
    )
  })
})
