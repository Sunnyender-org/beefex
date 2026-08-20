import { SettingRow, SettingsGroup } from './components'
import type { Lang } from './i18n'
import { aboutOpenSourceNoticeCopy } from './settingsSurface'

export function AboutOpenSourceNotice({ lang }: { lang: Lang }) {
  const copy = aboutOpenSourceNoticeCopy(lang)
  return (
    <SettingsGroup title={copy.title}>
      <SettingRow label={copy.label}>
        <span className="kv-row-desc">{copy.value}</span>
      </SettingRow>
    </SettingsGroup>
  )
}
