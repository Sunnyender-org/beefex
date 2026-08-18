import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { Progress } from './Progress'

describe('Progress', () => {
  it('keeps its public value inside the valid range', () => {
    const html = renderToStaticMarkup(<Progress value={140} label="Build" />)

    expect(html).toContain('aria-valuenow="100"')
    expect(html).toContain('width:100%')
  })
})
