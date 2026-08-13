import { describe, expect, it } from 'vitest'
import { PI_CAPABILITY_MAP } from './piCapabilities'

describe('Pi capability desktop mapping', () => {
  it('maps every command in the pinned Pi 0.84.1 RPC surface exactly once', () => {
    expect(PI_CAPABILITY_MAP).toHaveLength(32)
    expect(new Set(PI_CAPABILITY_MAP.map((item) => item.command)).size).toBe(32)
    expect(PI_CAPABILITY_MAP.every((item) => item.surface && item.behavior)).toBe(true)
  })

  it('keeps provider discovery behind the BeefAPI model picker', () => {
    for (const command of ['set_model', 'cycle_model', 'get_available_models']) {
      expect(PI_CAPABILITY_MAP.find((item) => item.command === command)?.surface).toBe('model-picker')
    }
  })

  it('keeps direct bash behind approval', () => {
    expect(PI_CAPABILITY_MAP.find((item) => item.command === 'bash')?.surface).toBe('approval')
  })
})
