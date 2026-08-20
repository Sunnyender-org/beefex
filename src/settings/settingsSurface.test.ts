import { describe, expect, it } from 'vitest'
import {
  DORMANT_GENERAL_SECTIONS,
  HIDDEN_SETTINGS_TABS,
  REQUIRED_GPL_ATTRIBUTION,
  VISIBLE_GENERAL_SECTIONS,
  VISIBLE_SETTINGS_TABS,
  aboutOpenSourceNoticeCopy,
  generalSettingsSubtitle,
  isHiddenSettingsTab,
  isVisibleGeneralSection,
  normalizeVisibleSettingsTab,
  resolveSettingsActiveTab,
  visibleSettingsNavItems,
} from './settingsSurface'

describe('visible Beefex settings tabs', () => {
  it('exposes only General, Usage/Diagnostics, Client integrations, and About', () => {
    expect([...VISIBLE_SETTINGS_TABS]).toEqual([
      'general',
      'usage',
      'clientIntegrations',
      'about',
    ])
    expect(visibleSettingsNavItems('zh').map((item) => item.id)).toEqual([
      ...VISIBLE_SETTINGS_TABS,
    ])
    expect(visibleSettingsNavItems('zh').map((item) => item.label)).toEqual([
      '基础',
      '用量统计',
      '客户端集成',
      '关于',
    ])
    expect(visibleSettingsNavItems('en').map((item) => item.label)).toEqual([
      'General',
      'Usage',
      'Client integrations',
      'About',
    ])
  })

  it('keeps dormant host-era product surfaces off the visible Settings route', () => {
    expect([...HIDDEN_SETTINGS_TABS]).toEqual([
      'hotkeys',
      'translate',
      'lens',
      'chat',
      'memory',
      'mixer',
      'mcp',
      'skill',
      'webSearch',
      'connectors',
      'knowledge',
      'providers',
    ])
    for (const tab of HIDDEN_SETTINGS_TABS) {
      expect(isHiddenSettingsTab(tab)).toBe(true)
      expect(VISIBLE_SETTINGS_TABS).not.toContain(tab)
    }
  })
})

describe('hidden initial-tab normalization', () => {
  it('defaults missing and hidden tabs to General for the normal Settings entry', () => {
    expect(normalizeVisibleSettingsTab(undefined)).toBe('general')
    expect(normalizeVisibleSettingsTab(null)).toBe('general')
    expect(normalizeVisibleSettingsTab('chat')).toBe('general')
    expect(normalizeVisibleSettingsTab('translate')).toBe('general')
    expect(normalizeVisibleSettingsTab('lens')).toBe('general')
    expect(normalizeVisibleSettingsTab('memory')).toBe('general')
    expect(normalizeVisibleSettingsTab('mixer')).toBe('general')
    expect(normalizeVisibleSettingsTab('mcp')).toBe('general')
    expect(normalizeVisibleSettingsTab('connectors')).toBe('general')
    expect(normalizeVisibleSettingsTab('knowledge')).toBe('general')
    expect(normalizeVisibleSettingsTab('skill')).toBe('general')
    expect(normalizeVisibleSettingsTab('webSearch')).toBe('general')
    expect(normalizeVisibleSettingsTab('providers')).toBe('general')
    expect(normalizeVisibleSettingsTab('hotkeys')).toBe('general')
    expect(normalizeVisibleSettingsTab('not-a-tab')).toBe('general')
  })

  it('keeps visible tabs and only preserves hidden tabs on the hideNav path', () => {
    expect(normalizeVisibleSettingsTab('usage')).toBe('usage')
    expect(normalizeVisibleSettingsTab('clientIntegrations')).toBe('clientIntegrations')
    expect(normalizeVisibleSettingsTab('about')).toBe('about')
    expect(resolveSettingsActiveTab('chat')).toBe('general')
    expect(resolveSettingsActiveTab('knowledge', true)).toBe('knowledge')
    expect(resolveSettingsActiveTab('general', true)).toBe('general')
  })
})

describe('visible General sections', () => {
  it('exposes only Appearance on the visible General page', () => {
    expect([...VISIBLE_GENERAL_SECTIONS]).toEqual(['appearance'])
    expect(isVisibleGeneralSection('appearance')).toBe(true)
    expect(generalSettingsSubtitle('zh')).toBe('界面语言、主题和颜色。')
    expect(generalSettingsSubtitle('en')).toBe('Language, theme, and color.')
    for (const section of DORMANT_GENERAL_SECTIONS) {
      expect(isVisibleGeneralSection(section)).toBe(false)
      expect(VISIBLE_GENERAL_SECTIONS).not.toContain(section)
    }
    expect([...DORMANT_GENERAL_SECTIONS]).toEqual([
      'behavior',
      'firstTimeSetup',
      'backupRestore',
      'macosPermissions',
    ])
  })
})

describe('About open-source attribution', () => {
  it('keeps the required GPL string as quiet notices, not upstream product framing', () => {
    const zh = aboutOpenSourceNoticeCopy('zh')
    const en = aboutOpenSourceNoticeCopy('en')
    expect(zh.value).toBe('Kivio · GPL-3.0-or-later')
    expect(en.value).toBe(REQUIRED_GPL_ATTRIBUTION)
    expect(zh.title).toBe('开源与许可')
    expect(en.title).toBe('Open-source licenses')
    expect(zh.label).toBe('第三方组件')
    expect(en.label).toBe('Third-party component')
    expect(zh.label).not.toBe(zh.title)
    expect(en.label).not.toBe(en.title)
    expect(`${zh.title} ${zh.label}`.toLowerCase()).not.toMatch(/upstream|上游/)
    expect(`${en.title} ${en.label}`.toLowerCase()).not.toMatch(/upstream|上游/)
  })
})
