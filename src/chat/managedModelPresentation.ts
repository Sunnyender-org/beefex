import { matchModel } from '../data/modelMatching'

export type ManagedModelFamily = {
  id: 'openai' | 'anthropic' | 'other'
  label: string
  featured: string[]
  secondary: string[]
  secondaryLabel: string
}

const normalizedModelId = (model: string) => model.toLowerCase().trim().split('/').pop() ?? ''

const isOpenAiModel = (model: string) => {
  const id = normalizedModelId(model)
  return id.startsWith('gpt-') || /^o\d(?:-|$)/.test(id)
}

const isAnthropicModel = (model: string) => normalizedModelId(model).startsWith('claude-')

const isGpt56Model = (model: string) => /^gpt-5[.-]6(?:-|$)/.test(normalizedModelId(model))

const anthropicFeaturedRank = (model: string) => {
  const id = normalizedModelId(model)
  if (/^claude-fable-5(?:-|$)/.test(id)) return 0
  if (/^claude-opus-5(?:-|$)/.test(id)) return 1
  if (/^claude-sonnet-5(?:-|$)/.test(id)) return 2
  return null
}

export function groupManagedModels(models: readonly string[]): ManagedModelFamily[] {
  const unique = [...new Set(models.filter(Boolean))]
  const openai = unique.filter(isOpenAiModel)
  const anthropic = unique.filter(isAnthropicModel)
  const other = unique.filter((model) => !isOpenAiModel(model) && !isAnthropicModel(model))

  const anthropicFeatured = anthropic
    .map((model, sourceIndex) => ({ model, sourceIndex, rank: anthropicFeaturedRank(model) }))
    .filter((entry): entry is { model: string; sourceIndex: number; rank: number } => entry.rank !== null)
    .sort((a, b) => a.rank - b.rank || a.sourceIndex - b.sourceIndex)
    .map(({ model }) => model)

  const featuredAnthropic = new Set(anthropicFeatured)
  const families: ManagedModelFamily[] = [
    {
      id: 'openai',
      label: 'OPENAI',
      featured: openai.filter(isGpt56Model),
      secondary: openai.filter((model) => !isGpt56Model(model)),
      secondaryLabel: '其他 OpenAI 模型',
    },
    {
      id: 'anthropic',
      label: 'ANTHROPIC',
      featured: anthropicFeatured,
      secondary: anthropic.filter((model) => !featuredAnthropic.has(model)),
      secondaryLabel: '其他 Anthropic 模型',
    },
  ]

  if (other.length > 0) {
    families.push({
      id: 'other',
      label: '其他',
      featured: [],
      secondary: other,
      secondaryLabel: '其他模型',
    })
  }

  return families.filter((family) => family.featured.length > 0 || family.secondary.length > 0)
}

export function managedModelDisplayName(model: string) {
  const matched = matchModel(model)?.displayName
  if (matched) return matched
  return normalizedModelId(model)
    .split('-')
    .filter(Boolean)
    .map((part, index) => {
      if (part === 'gpt') return 'GPT'
      if (index === 0 || part === 'fable' || part === 'opus' || part === 'sonnet') {
        return `${part.charAt(0).toUpperCase()}${part.slice(1)}`
      }
      return part
    })
    .join(' ')
}
