export function resolveManagedModelValue(
  draftModel: string,
  defaultModel: string | undefined,
  allowedModels: readonly string[],
): string {
  if (draftModel && allowedModels.includes(draftModel)) return draftModel
  if (defaultModel && allowedModels.includes(defaultModel)) return defaultModel
  return allowedModels[0] ?? defaultModel ?? draftModel
}
