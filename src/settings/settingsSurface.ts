import { i18n, type Lang } from './i18n'

export const VISIBLE_SETTINGS_TABS = ['general', 'usage', 'clientIntegrations', 'about'] as const
export type VisibleSettingsTab = (typeof VISIBLE_SETTINGS_TABS)[number]

export const HIDDEN_SETTINGS_TABS = [
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
] as const
export type HiddenSettingsTab = (typeof HIDDEN_SETTINGS_TABS)[number]

export type SettingsTab = VisibleSettingsTab | HiddenSettingsTab

export const REQUIRED_GPL_ATTRIBUTION = 'Kivio · GPL-3.0-or-later'

export function isVisibleSettingsTab(tab: string | null | undefined): tab is VisibleSettingsTab {
  return tab != null && (VISIBLE_SETTINGS_TABS as readonly string[]).includes(tab)
}

export function isHiddenSettingsTab(tab: string | null | undefined): tab is HiddenSettingsTab {
  return tab != null && (HIDDEN_SETTINGS_TABS as readonly string[]).includes(tab)
}

/** Normal Settings entry: hidden or unknown tabs become General. */
export function normalizeVisibleSettingsTab(
  tab: string | null | undefined,
): VisibleSettingsTab {
  return isVisibleSettingsTab(tab) ? tab : 'general'
}

/**
 * hideNav is the dormant single-page extension path (e.g. Knowledge).
 * Normal Settings always normalizes hidden initialTab values to General.
 */
export function resolveSettingsActiveTab(
  tab: SettingsTab | null | undefined,
  hideNav = false,
): SettingsTab {
  if (hideNav && tab) return tab
  return normalizeVisibleSettingsTab(tab)
}

export function visibleSettingsNavItems(lang: Lang): Array<{ id: VisibleSettingsTab; label: string }> {
  const t = i18n[lang]
  return [
    { id: 'general', label: t.tabGeneral },
    { id: 'usage', label: lang === 'zh' ? '用量统计' : 'Usage' },
    { id: 'clientIntegrations', label: lang === 'zh' ? '客户端集成' : 'Client integrations' },
    { id: 'about', label: lang === 'zh' ? '关于' : 'About' },
  ]
}

export function visibleSettingsPrimaryNavItems(lang: Lang): Array<{
  id: Exclude<VisibleSettingsTab, 'about'>
  label: string
}> {
  return visibleSettingsNavItems(lang).filter(
    (item): item is { id: Exclude<VisibleSettingsTab, 'about'>; label: string } =>
      item.id !== 'about',
  )
}

export const VISIBLE_GENERAL_SECTIONS = ['appearance'] as const
export type VisibleGeneralSection = (typeof VISIBLE_GENERAL_SECTIONS)[number]

export const DORMANT_GENERAL_SECTIONS = [
  'behavior',
  'firstTimeSetup',
  'backupRestore',
  'macosPermissions',
] as const
export type DormantGeneralSection = (typeof DORMANT_GENERAL_SECTIONS)[number]
export type GeneralSection = VisibleGeneralSection | DormantGeneralSection

export function isVisibleGeneralSection(section: GeneralSection): boolean {
  return (VISIBLE_GENERAL_SECTIONS as readonly string[]).includes(section)
}

export function generalSettingsSubtitle(lang: Lang): string {
  return lang === 'zh' ? '界面语言、主题和颜色。' : 'Language, theme, and color.'
}

export function aboutOpenSourceNoticeCopy(lang: Lang): {
  title: string
  label: string
  value: typeof REQUIRED_GPL_ATTRIBUTION
} {
  return {
    title: lang === 'zh' ? '开源与许可' : 'Open-source licenses',
    label: lang === 'zh' ? '第三方组件' : 'Third-party component',
    value: REQUIRED_GPL_ATTRIBUTION,
  }
}
