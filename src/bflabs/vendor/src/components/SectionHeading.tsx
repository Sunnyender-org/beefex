import type { HTMLAttributes, ReactNode } from 'react'
import { cx } from '../lib/cx'

export type SectionHeadingProps = HTMLAttributes<HTMLDivElement> & {
  index?: string
  label?: string
  eyebrow?: string
  title: ReactNode
  summary?: ReactNode
}

export function SectionHeading({
  index,
  label,
  eyebrow,
  title,
  summary,
  className,
  ...props
}: SectionHeadingProps) {
  return (
    <div
      className={cx('bf-section-heading', className)}
      data-slot="section-heading"
      {...props}
    >
      <div className="bf-section-heading__meta">
        <span className="bf-section-heading__index">{index}</span>
        <span>{label}</span>
      </div>
      <div>
        {eyebrow ? (
          <span className="bf-section-heading__eyebrow">{eyebrow}</span>
        ) : null}
        <h2 className="bf-section-heading__title">{title}</h2>
      </div>
      {summary ? <p className="bf-section-heading__summary">{summary}</p> : null}
    </div>
  )
}
