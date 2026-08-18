import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { BrandMark } from './vendor/src/components/Brand'

describe('canonical BFLabs integration', () => {
  it('renders the canonical brand mark primitive', () => {
    render(<BrandMark label="BF Labs" />)

    expect(screen.getByRole('img', { name: 'BF Labs' })).toHaveAttribute('data-slot', 'brand-mark')
  })
})
