import {
  useMemo,
  useRef,
  type CSSProperties,
  type HTMLAttributes,
  type PointerEvent,
} from 'react'
import { cx } from '../lib/cx'

export type KineticHeadingLine = {
  text: string
  tone?: 'default' | 'muted' | 'accent'
}

export type KineticHeadingProps = Omit<
  HTMLAttributes<HTMLHeadingElement>,
  'children'
> & {
  as?: 'h1' | 'h2' | 'h3'
  lines: readonly KineticHeadingLine[]
  influenceRadius?: number
}

type WordStyle = CSSProperties & {
  '--bf-word-scale'?: string
  '--bf-word-shift'?: string
  '--bf-word-opacity'?: string
}

export function KineticHeading({
  as: Tag = 'h2',
  lines,
  influenceRadius = 230,
  className,
  onPointerMove,
  onPointerLeave,
  ...props
}: KineticHeadingProps) {
  const rootRef = useRef<HTMLHeadingElement>(null)
  const frameRef = useRef<number | null>(null)
  const pointerRef = useRef({ x: 0, y: 0 })

  const indexedLines = useMemo(() => {
    let index = 0
    return lines.map((line) => ({
      ...line,
      words: line.text.trim().split(/\s+/).map((word) => ({ word, index: index++ })),
    }))
  }, [lines])

  const reset = () => {
    rootRef.current
      ?.querySelectorAll<HTMLElement>('[data-bf-kinetic-word]')
      .forEach((word) => {
        word.dataset.active = 'false'
        word.style.removeProperty('--bf-word-scale')
        word.style.removeProperty('--bf-word-shift')
        word.style.removeProperty('--bf-word-opacity')
      })
  }

  const update = () => {
    const root = rootRef.current
    if (!root) return

    const words = Array.from(
      root.querySelectorAll<HTMLElement>('[data-bf-kinetic-word]'),
    )
    const distances = words.map((word) => {
      const bounds = word.getBoundingClientRect()
      return Math.hypot(
        pointerRef.current.x - (bounds.left + bounds.width / 2),
        pointerRef.current.y - (bounds.top + bounds.height / 2),
      )
    })
    const closest = Math.min(...distances)

    if (closest > influenceRadius) {
      reset()
      return
    }

    words.forEach((word, index) => {
      const active = distances[index] === closest
      const pressure = Math.max(0, 1 - distances[index] / influenceRadius)
      word.dataset.active = String(active)
      word.style.setProperty('--bf-word-scale', active ? '1.035' : '1')
      word.style.setProperty('--bf-word-shift', active ? '0.45rem' : '0')
      word.style.setProperty(
        '--bf-word-opacity',
        active ? '1' : Math.max(0.7, 1 - pressure * 0.3).toFixed(3),
      )
    })
  }

  const handlePointerMove = (event: PointerEvent<HTMLHeadingElement>) => {
    onPointerMove?.(event)
    if (event.defaultPrevented || event.pointerType === 'touch') return

    pointerRef.current = { x: event.clientX, y: event.clientY }
    if (frameRef.current === null) {
      frameRef.current = window.requestAnimationFrame(() => {
        update()
        frameRef.current = null
      })
    }
  }

  const handlePointerLeave = (event: PointerEvent<HTMLHeadingElement>) => {
    onPointerLeave?.(event)
    reset()
  }

  return (
    <Tag
      ref={rootRef}
      className={cx('bf-kinetic-heading', className)}
      data-slot="kinetic-heading"
      onPointerMove={handlePointerMove}
      onPointerLeave={handlePointerLeave}
      {...props}
    >
      {indexedLines.map((line, lineIndex) => (
        <span
          className={cx(
            'bf-kinetic-heading__line',
            `bf-kinetic-heading__line--${line.tone ?? 'default'}`,
          )}
          key={`${line.text}-${lineIndex}`}
        >
          {line.words.map(({ word, index }, wordIndex) => (
            <span key={`${word}-${index}`}>
              <span
                className="bf-kinetic-heading__word"
                data-bf-kinetic-word
                data-active="false"
                style={{} as WordStyle}
              >
                {word}
              </span>
              {wordIndex < line.words.length - 1 ? ' ' : null}
            </span>
          ))}
        </span>
      ))}
    </Tag>
  )
}
