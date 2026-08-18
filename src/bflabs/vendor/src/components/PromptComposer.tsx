import {
  useState,
  type FormEvent,
  type KeyboardEvent,
  type TextareaHTMLAttributes,
} from 'react'
import { cx } from '../lib/cx'
import { ArrowRightIcon } from './icons'

export type PromptComposerProps = Omit<
  TextareaHTMLAttributes<HTMLTextAreaElement>,
  'value' | 'defaultValue' | 'onChange' | 'onSubmit'
> & {
  value?: string
  defaultValue?: string
  onValueChange?: (value: string) => void
  onSubmit?: (value: string) => void
  submitLabel?: string
  className?: string
}

export function PromptComposer({
  value,
  defaultValue = '',
  onValueChange,
  onSubmit,
  submitLabel = 'Send message',
  placeholder = 'Describe the outcome you want to move forward…',
  disabled,
  className,
  ...props
}: PromptComposerProps) {
  const [internalValue, setInternalValue] = useState(defaultValue)
  const currentValue = value ?? internalValue

  const updateValue = (nextValue: string) => {
    if (value === undefined) setInternalValue(nextValue)
    onValueChange?.(nextValue)
  }

  const submit = () => {
    const nextValue = currentValue.trim()
    if (!nextValue || disabled) return
    onSubmit?.(nextValue)
    updateValue('')
  }

  const handleSubmit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    submit()
  }

  const handleKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    props.onKeyDown?.(event)
    if (event.defaultPrevented) return
    if (event.key === 'Enter' && !event.shiftKey) {
      event.preventDefault()
      submit()
    }
  }

  return (
    <form
      className={cx('bf-prompt-composer', className)}
      data-slot="prompt-composer"
      onSubmit={handleSubmit}
    >
      <textarea
        className="bf-prompt-composer__input"
        value={currentValue}
        placeholder={placeholder}
        disabled={disabled}
        rows={1}
        onChange={(event) => updateValue(event.target.value)}
        {...props}
        onKeyDown={handleKeyDown}
      />
      <button
        className="bf-prompt-composer__submit"
        type="submit"
        disabled={disabled || currentValue.trim().length === 0}
        aria-label={submitLabel}
      >
        <ArrowRightIcon />
      </button>
    </form>
  )
}
