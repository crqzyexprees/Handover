/** Last segment of a path (Windows or POSIX). */
export function folderNameFromPath(path) {
  if (path == null || path === '') return 'Unknown'
  const s = String(path).replace(/\\/g, '/').replace(/\/+$/, '')
  const parts = s.split('/').filter(Boolean)
  return parts.length ? parts[parts.length - 1] : 'Unknown'
}

/** User-facing project title for sidebar and UI. */
export function getProjectDisplayName(project) {
  if (!project) return ''
  return (
    project.config.project_name?.trim() ||
    project.name?.trim() ||
    folderNameFromPath(project.path)
  )
}

/** Sidebar label for a terminal row under a project. */
export function getTerminalSidebarLabel(instance, index) {
  const mode = instance.sandbox_mode === 'native' ? 'Native' : 'Docker'
  return `Terminal ${index + 1} (${mode})`
}
