import { forwardRef, type HTMLAttributes } from 'react'
import { cx } from '../lib/cx'

export type BFThemeTone = 'light' | 'dark' | 'orange'

export type BFThemeProps = HTMLAttributes<HTMLDivElement> & {
  tone?: BFThemeTone
}

export const BFTheme = forwardRef<HTMLDivElement, BFThemeProps>(
  ({ tone = 'light', className, ...props }, ref) => (
    <div
      ref={ref}
      className={cx('bf-theme', className)}
      data-slot="theme"
      data-tone={tone}
      {...props}
    />
  ),
)

BFTheme.displayName = 'BFTheme'

export const BFProvider = BFTheme
export type BFProviderProps = BFThemeProps
