import type { HTMLAttributes, ReactNode } from 'react'
import { cx } from '../lib/cx'
import { BrandMark } from './Brand'

export type AgentMessageRole = 'assistant' | 'user' | 'system'

export type AgentMessageProps = HTMLAttributes<HTMLElement> & {
  role?: AgentMessageRole
  label?: string
  meta?: ReactNode
}

export function AgentMessage({
  role = 'assistant',
  label,
  meta,
  className,
  children,
  ...props
}: AgentMessageProps) {
  const resolvedLabel =
    label ?? (role === 'assistant' ? 'BF Agent' : role === 'user' ? 'You' : 'System')

  return (
    <article
      className={cx('bf-agent-message', `bf-agent-message--${role}`, className)}
      data-slot="agent-message"
      {...props}
    >
      <div className="bf-agent-message__avatar" aria-hidden="true">
        {role === 'assistant' ? (
          <BrandMark />
        ) : (
          <span className="bf-agent-message__avatar-dot" />
        )}
      </div>
      <div className="bf-agent-message__content">
        <div className="bf-agent-message__meta">
          <span>{resolvedLabel}</span>
          {meta ? <span>{meta}</span> : null}
        </div>
        <div className="bf-agent-message__body">{children}</div>
      </div>
    </article>
  )
}

export function AgentThread({
  className,
  ...props
}: HTMLAttributes<HTMLDivElement>) {
  return (
    <div
      className={cx('bf-agent-thread', className)}
      data-slot="agent-thread"
      {...props}
    />
  )
}
