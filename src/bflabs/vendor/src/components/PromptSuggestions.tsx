import type { HTMLAttributes } from 'react'
import { cx } from '../lib/cx'
import { ArrowRightIcon } from './icons'

export type PromptSuggestion = {
  id: string
  label: string
  description?: string
}

export type PromptSuggestionsProps = Omit<
  HTMLAttributes<HTMLDivElement>,
  'onSelect'
> & {
  items: readonly PromptSuggestion[]
  onSelect?: (item: PromptSuggestion) => void
  label?: string
}

export function PromptSuggestions({
  items,
  onSelect,
  label = 'Suggested prompts',
  className,
  ...props
}: PromptSuggestionsProps) {
  return (
    <div
      className={cx('bf-prompt-suggestions', className)}
      data-slot="prompt-suggestions"
      aria-label={label}
      {...props}
    >
      {items.map((item) => (
        <button
          className="bf-prompt-suggestion"
          type="button"
          key={item.id}
          onClick={() => onSelect?.(item)}
        >
          <span>
            <strong>{item.label}</strong>
            {item.description ? <small>{item.description}</small> : null}
          </span>
          <ArrowRightIcon />
        </button>
      ))}
    </div>
  )
}
