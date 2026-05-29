/** Last segment of a path (Windows or POSIX). */
export function folderNameFromPath(path) {
  if (path == null || path === '') return 'Unknown'
  const s = String(path).replace(/\\/g, '/').replace(/\/+$/, '')
  const parts = s.split('/').filter(Boolean)
  return parts.length ? parts[parts.length - 1] : 'Unknown'
}

export function getProjectId(project) {
  if (!project || typeof project !== 'object') return null
  return project.id ?? project.project_id ?? project.projectId ?? null
}

export function getInstanceId(instance) {
  if (!instance || typeof instance !== 'object') return null
  return instance.id ?? instance.instance_id ?? instance.instanceId ?? null
}

export function normalizeProjectStatus(status) {
  const s = String(status ?? 'UNLOADED').toUpperCase()
  if (s === 'ACTIVE') return 'ACTIVE'
  if (s === 'SUSPENDED') return 'SUSPENDED'
  return 'UNLOADED'
}

export function getProjectInstances(project) {
  if (!project || typeof project !== 'object') return []
  const raw = project.instances ?? project.instance_list ?? []
  return Array.isArray(raw) ? raw : []
}

export function instanceIsActiveWriter(instance) {
  if (!instance || typeof instance !== 'object') return false
  return Boolean(
    instance.activeWriter ??
      instance.isActiveWriter ??
      instance.active_writer ??
      instance.meta?.activeWriter,
  )
}

const CLI_LABELS = {
  claude: 'Claude',
  codex: 'Codex',
  gemini: 'Gemini',
  cursor: 'Cursor',
}

export function getProjectConfig(project) {
  if (!project?.config || typeof project.config !== 'object') {
    return null
  }
  return project.config
}

/** User-facing project title for sidebar and UI. */
export function getProjectDisplayName(project) {
  const config = getProjectConfig(project)
  const fromConfig = config?.project_name?.trim()
  if (fromConfig) return fromConfig
  const fromProject = project?.name?.trim()
  if (fromProject) return fromProject
  return folderNameFromPath(
    project?.path ?? project?.root ?? project?.project_path ?? '',
  )
}

/** Sidebar label for a terminal row under a project. */
export function getTerminalSidebarLabel(project, instance, index) {
  const mode = instance?.sandbox_mode === 'native' ? 'Native' : 'Docker'
  return `Terminal ${index + 1} (${mode})`
}

export function instanceDisplayLabel(instance) {
  const raw = instance?.cliType ?? instance?.cli_type ?? instance?.type ?? ''
  const key = String(raw).toLowerCase()
  if (CLI_LABELS[key]) return CLI_LABELS[key]
  if (!key) return 'CLI'
  return key.charAt(0).toUpperCase() + key.slice(1)
}
