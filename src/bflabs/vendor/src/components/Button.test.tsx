import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { Button } from './Button'

describe('Button', () => {
  it('renders the selected variant and native attributes', () => {
    const html = renderToStaticMarkup(
      <Button variant="accent" disabled trailingIcon>
        Continue
      </Button>,
    )

    expect(html).toContain('bf-button--accent')
    expect(html).toContain('disabled=""')
    expect(html).toContain('Continue')
  })

  it('announces a loading state', () => {
    const html = renderToStaticMarkup(<Button loading>Working</Button>)

    expect(html).toContain('aria-busy="true"')
    expect(html).toContain('bf-button__spinner')
  })
})
