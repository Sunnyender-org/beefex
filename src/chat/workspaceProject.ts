export type WorkspaceProjectRef = {
  id: string
  name: string
}

/**
 * Titlebar / composer project truth: selected navigation project first,
 * then the current conversation's persisted project. Nameless or missing
 * conversation projects stay unbound.
 */
export function resolveWorkspaceProject<T extends WorkspaceProjectRef>(
  selectedProject: T | null | undefined,
  conversationProject: WorkspaceProjectRef | null | undefined,
): T | WorkspaceProjectRef | null {
  if (selectedProject) return selectedProject
  if (conversationProject?.name) return conversationProject
  return null
}
