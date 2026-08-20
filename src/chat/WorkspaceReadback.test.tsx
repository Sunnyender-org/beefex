import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { WorkspaceReadback } from './WorkspaceReadback'
import { resolveWorkspaceProject } from './workspaceProject'

const selected = { id: 'nav', name: 'Nav Project' }
const recovered = { id: 'task', name: 'Recovered Task' }

describe('resolveWorkspaceProject', () => {
  it('uses selectedProject first, then the persisted conversation project', () => {
    expect(resolveWorkspaceProject(selected, recovered)).toEqual(selected)
    expect(resolveWorkspaceProject(null, recovered)).toEqual(recovered)
    expect(resolveWorkspaceProject(undefined, { id: 'empty', name: '' })).toBeNull()
    expect(resolveWorkspaceProject(null, null)).toBeNull()
  })
})

describe('WorkspaceReadback', () => {
  it('shows the persisted conversation project when navigation selection is empty', () => {
    render(
      <WorkspaceReadback
        selectedProject={null}
        conversationProject={recovered}
        lang="zh"
      />,
    )
    expect(screen.getByText('工作区')).toBeInTheDocument()
    expect(screen.getByText('Recovered Task')).toBeInTheDocument()
    expect(screen.queryByText('未选择项目')).not.toBeInTheDocument()
    expect(screen.getByTitle('Recovered Task')).toBeInTheDocument()
  })

  it('keeps the unselected fallback for a truly unbound Task', () => {
    render(
      <WorkspaceReadback
        selectedProject={null}
        conversationProject={null}
        lang="zh"
      />,
    )
    expect(screen.getByText('未选择项目')).toBeInTheDocument()
    expect(screen.getByTitle('未选择项目')).toBeInTheDocument()
  })

  it('lets an explicit navigation selection win over the conversation project', () => {
    render(
      <WorkspaceReadback
        selectedProject={selected}
        conversationProject={recovered}
        lang="en"
      />,
    )
    expect(screen.getByText('WORKSPACE')).toBeInTheDocument()
    expect(screen.getByText('Nav Project')).toBeInTheDocument()
    expect(screen.queryByText('Recovered Task')).not.toBeInTheDocument()
    expect(screen.queryByText('No project')).not.toBeInTheDocument()
  })
})
