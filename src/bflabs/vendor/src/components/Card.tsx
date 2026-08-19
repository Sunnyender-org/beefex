import type { HTMLAttributes, ReactNode } from 'react'
import { cx } from '../lib/cx'

export type CardTone = 'default' | 'dark' | 'accent'

export type CardProps = HTMLAttributes<HTMLElement> & {
  as?: 'article' | 'div' | 'section'
  tone?: CardTone
  interactive?: boolean
  index?: string
  title?: ReactNode
  description?: ReactNode
}

export function Card({
  as: Tag = 'article',
  tone = 'default',
  interactive = false,
  index,
  title,
  description,
  className,
  children,
  ...props
}: CardProps) {
  return (
    <Tag
      className={cx(
        'bf-card',
        `bf-card--${tone}`,
        interactive && 'bf-card--interactive',
        className,
      )}
      data-slot="card"
      {...props}
    >
      {index ? <span className="bf-card__index">{index}</span> : null}
      {title ? <h3 className="bf-card__title">{title}</h3> : null}
      {description ? <p className="bf-card__body">{description}</p> : null}
      {children}
    </Tag>
  )
}
