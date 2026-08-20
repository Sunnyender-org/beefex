import { resolveWorkspaceProject, type WorkspaceProjectRef } from './workspaceProject'

export function WorkspaceReadback({
  selectedProject = null,
  conversationProject = null,
  lang,
}: {
  selectedProject?: WorkspaceProjectRef | null
  conversationProject?: WorkspaceProjectRef | null
  lang: 'zh' | 'en'
}) {
  const project = resolveWorkspaceProject(selectedProject, conversationProject)
  const unboundLabel = lang === 'en' ? 'No project' : '未选择项目'
  const unboundTitle = lang === 'en' ? 'No project selected' : '未选择项目'
  return (
    <div className="beef-workspace-readback" title={project?.name || unboundTitle}>
      <span>{lang === 'en' ? 'WORKSPACE' : '工作区'}</span>
      <strong>{project?.name || unboundLabel}</strong>
    </div>
  )
}
