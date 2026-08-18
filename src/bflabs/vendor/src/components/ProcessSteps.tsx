import type { CSSProperties, HTMLAttributes } from 'react'
import { cx } from '../lib/cx'

export type ProcessStepState = 'queued' | 'active' | 'complete'

export type ProcessStepItem = {
  id: string
  label: string
  description?: string
  state?: ProcessStepState
}

export type ProcessStepsProps = Omit<HTMLAttributes<HTMLOListElement>, 'children'> & {
  items: readonly ProcessStepItem[]
  value?: string
  onValueChange?: (value: string) => void
}

type ProcessStyle = CSSProperties & {
  '--bf-step-count': number
}

export function ProcessSteps({
  items,
  value,
  onValueChange,
  className,
  style,
  ...props
}: ProcessStepsProps) {
  return (
    <ol
      className={cx('bf-process-steps', className)}
      data-slot="process-steps"
      style={{ '--bf-step-count': items.length, ...style } as ProcessStyle}
      {...props}
    >
      {items.map((item, index) => {
        const state = value === item.id ? 'active' : item.state ?? 'queued'
        const content = (
          <>
            <span className="bf-process-step__number">
              {String(index + 1).padStart(2, '0')}
            </span>
            <strong className="bf-process-step__label">{item.label}</strong>
            {item.description ? (
              <span className="bf-process-step__description">{item.description}</span>
            ) : null}
          </>
        )

        return (
          <li className="bf-process-step" data-state={state} key={item.id}>
            {onValueChange ? (
              <button
                className="bf-process-step__button"
                type="button"
                aria-current={state === 'active' ? 'step' : undefined}
                onClick={() => onValueChange(item.id)}
              >
                {content}
              </button>
            ) : (
              <div className="bf-process-step__static">{content}</div>
            )}
          </li>
        )
      })}
    </ol>
  )
}
