import type { HTMLAttributes, ImgHTMLAttributes } from 'react'
import markUrl from '../assets/bf-mark.svg'
import { cx } from '../lib/cx'

export type BrandMarkProps = Omit<
  ImgHTMLAttributes<HTMLImageElement>,
  'src' | 'alt'
> & {
  label?: string
}

export function BrandMark({ label, className, ...props }: BrandMarkProps) {
  return (
    <img
      className={cx('bf-brand-mark', className)}
      data-slot="brand-mark"
      src={markUrl}
      alt={label ?? ''}
      aria-hidden={label ? undefined : true}
      {...props}
    />
  )
}

export type BrandLockupProps = HTMLAttributes<HTMLDivElement> & {
  orientation?: 'horizontal' | 'vertical'
  tagline?: string | false
}

export function BrandLockup({
  orientation = 'horizontal',
  tagline = 'Build Forward with AI Agents',
  className,
  ...props
}: BrandLockupProps) {
  return (
    <div
      className={cx(
        'bf-brand-lockup',
        `bf-brand-lockup--${orientation}`,
        className,
      )}
      data-slot="brand-lockup"
      {...props}
    >
      <BrandMark />
      <span className="bf-brand-lockup__copy">
        <span className="bf-brand-lockup__name">BF LABS</span>
        {tagline ? (
          <span className="bf-brand-lockup__tagline">{tagline}</span>
        ) : null}
      </span>
    </div>
  )
}
