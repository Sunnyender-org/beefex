import type { HTMLAttributes, ReactNode } from 'react'
import { cx } from '../lib/cx'
import { Button } from './Button'

export type ConfirmationDetail = {
  label: string
  value: ReactNode
}

export type ConfirmationCardProps = HTMLAttributes<HTMLElement> & {
  eyebrow?: string
  title: ReactNode
  description?: ReactNode
  details?: readonly ConfirmationDetail[]
  confirmLabel?: string
  cancelLabel?: string
  onConfirm?: () => void
  onCancel?: () => void
  pending?: boolean
}

export function ConfirmationCard({
  eyebrow = 'Review before continuing',
  title,
  description,
  details = [],
  confirmLabel = 'Confirm',
  cancelLabel = 'Cancel',
  onConfirm,
  onCancel,
  pending = false,
  className,
  ...props
}: ConfirmationCardProps) {
  return (
    <article
      className={cx('bf-confirmation-card', className)}
      data-slot="confirmation-card"
      {...props}
    >
      <span className="bf-confirmation-card__eyebrow">{eyebrow}</span>
      <h3 className="bf-confirmation-card__title">{title}</h3>
      {description ? (
        <p className="bf-confirmation-card__description">{description}</p>
      ) : null}
      {details.length ? (
        <dl className="bf-confirmation-card__details">
          {details.map((detail) => (
            <div key={detail.label}>
              <dt>{detail.label}</dt>
              <dd>{detail.value}</dd>
            </div>
          ))}
        </dl>
      ) : null}
      <div className="bf-confirmation-card__actions">
        <Button variant="primary" loading={pending} onClick={onConfirm}>
          {confirmLabel}
        </Button>
        <Button variant="quiet" disabled={pending} onClick={onCancel}>
          {cancelLabel}
        </Button>
      </div>
    </article>
  )
}
