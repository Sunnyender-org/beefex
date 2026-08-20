import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { AboutOpenSourceNotice } from './AboutOpenSourceNotice'

describe('AboutOpenSourceNotice', () => {
  it('renders quiet GPL attribution without upstream framing', () => {
    const { rerender } = render(<AboutOpenSourceNotice lang="zh" />)
    expect(screen.getByText('开源与许可')).toBeInTheDocument()
    expect(screen.getByText('第三方组件')).toBeInTheDocument()
    expect(screen.getByText('Kivio · GPL-3.0-or-later')).toBeInTheDocument()
    expect(screen.queryByText('开源声明')).not.toBeInTheDocument()
    expect(screen.queryByText('上游项目')).not.toBeInTheDocument()
    expect(screen.queryByText('Upstream')).not.toBeInTheDocument()
    expect(screen.queryByText(/upstream project/i)).not.toBeInTheDocument()

    rerender(<AboutOpenSourceNotice lang="en" />)
    expect(screen.getByText('Open-source licenses')).toBeInTheDocument()
    expect(screen.getByText('Third-party component')).toBeInTheDocument()
    expect(screen.getByText('Kivio · GPL-3.0-or-later')).toBeInTheDocument()
    expect(screen.queryByText('Open-source notices')).not.toBeInTheDocument()
    expect(screen.queryByText('Upstream')).not.toBeInTheDocument()
  })
})
