import type { HTMLAttributes } from 'react'
import { cx } from '../lib/cx'

export type ProgressProps = HTMLAttributes<HTMLDivElement> & {
  value: number
  label?: string
  showValue?: boolean
}

export function Progress({
  value,
  label,
  showValue = true,
  className,
  ...props
}: ProgressProps) {
  const normalized = Math.min(100, Math.max(0, value))

  return (
    <div className={cx('bf-progress', className)} data-slot="progress" {...props}>
      {label || showValue ? (
        <div className="bf-progress__meta">
          <span>{label}</span>
          {showValue ? <span>{Math.round(normalized)}%</span> : null}
        </div>
      ) : null}
      <div
        className="bf-progress__track"
        role="progressbar"
        aria-label={label}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={normalized}
      >
        <div className="bf-progress__value" style={{ width: `${normalized}%` }} />
      </div>
    </div>
  )
}
