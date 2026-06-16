import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import * as api from './api.js'
import {
  getInstanceId,
  getProjectDisplayName,
  getProjectId,
  getProjectInstances,
} from './projectUtils.js'
import HandoverModal from './HandoverModal.jsx'
import ProjectSettings from './ProjectSettings.jsx'
import Sidebar from './Sidebar.jsx'
import TabBar from './TabBar.jsx'
import TerminalView from './TerminalView.jsx'

const DEFAULT_PROJECT_CONFIG = {
  project_name: '',
  sandbox_mode: 'docker',
  mem_limit: '2g',
  handoff_method: 'summary',
  custom_env_vars: {},
}

function formatApiError(error) {
  if (error == null) return 'Unknown error'
  if (typeof error === 'string') return error
  if (typeof error !== 'object') return 'Request failed'
  if (typeof error.detail === 'string') return error.detail
  if (Array.isArray(error.detail)) {
    return error.detail
      .map((d) => (typeof d === 'object' && d?.msg ? d.msg : String(d)))
      .join('; ')
  }
  if (error.message) return String(error.message)
  if (error.httpStatus) return `Request failed (${error.httpStatus})`
  if (typeof error.status === 'number') return `Request failed (${error.status})`
  return 'Request failed'
}

const TOAST_VARIANT_CLASS = {
  error: 'border-red-800/60 bg-red-950/50 text-red-200',
  success: 'border-green-800/60 bg-green-950/50 text-green-200',
  warn: 'border-yellow-800/60 bg-yellow-950/50 text-yellow-200',
  info: 'border-[#333333] bg-[#252526] text-[#cccccc]',
}

function ToastStack({ toasts }) {
  return (
    <div className="pointer-events-none fixed bottom-4 right-4 z-50 flex max-w-sm flex-col gap-2">
      {toasts.map((t) => (
        <div
          key={t.id}
          className={`pointer-events-auto rounded-sm border px-3 py-2 text-sm shadow-lg ${
            TOAST_VARIANT_CLASS[t.variant] ?? TOAST_VARIANT_CLASS.info
          }`}
        >
          {t.message}
        </div>
      ))}
    </div>
  )
}

export default function App() {
  const [projects, setProjects] = useState([])
  const [focusedProjectId, setFocusedProjectId] = useState(null)
  const [focusedInstanceId, setFocusedInstanceId] = useState(null)
  const [toasts, setToasts] = useState([])
  const [handoverOpen, setHandoverOpen] = useState(false)
  const [isSettingsModalOpen, setIsSettingsModalOpen] = useState(false)
  const [settingsProjectId, setSettingsProjectId] = useState(null)
  const [sidebarOpen, setSidebarOpen] = useState(true)
  const [ptyConnections, setPtyConnections] = useState({})
  const restoreToastShownRef = useRef(false)

  const pushToast = useCallback((message, variant = 'error') => {
    const id =
      globalThis.crypto?.randomUUID?.() ??
      `${Date.now()}-${Math.random().toString(16).slice(2)}`
    setToasts((prev) => [...prev, { id, message, variant }])
    window.setTimeout(() => {
      setToasts((prev) => prev.filter((x) => x.id !== id))
    }, 5000)
  }, [])

  const handlePtyConnectionChange = useCallback((instanceId, status) => {
    const id = String(instanceId ?? '')
    if (id === '') return
    setPtyConnections((prev) => {
      if (prev[id] === status) return prev
      return { ...prev, [id]: status }
    })
  }, [])

  const refreshProjects = useCallback(async () => {
    const { data, error } = await api.listProjects()
    if (error) {
      pushToast(`Could not load projects: ${formatApiError(error)}`, 'error')
      return null
    }
    if (data != null) {
      const incoming = Array.isArray(data) ? data : []
      let merged = incoming
      setProjects((prev) => {
        const prevById = new Map(
          prev.map((project) => [String(getProjectId(project) ?? ''), project]),
        )
        merged = incoming.map((project) => {
          const id = String(getProjectId(project) ?? '')
          const previous = prevById.get(id)
          const hasInstancesFromApi =
            Array.isArray(project.instances) || Array.isArray(project.instance_list)
          const hasConfigFromApi =
            project?.config && typeof project.config === 'object'
          if ((hasInstancesFromApi && hasConfigFromApi) || !previous) {
            return project
          }
          const preserved = getProjectInstances(previous)
          const nextProject = preserved.length > 0
            ? { ...project, instances: preserved }
            : project
          if (!hasConfigFromApi && previous?.config) {
            return {
              ...nextProject,
              config: previous.config,
            }
          }
          return nextProject
        })
        return merged
      })
      if (!restoreToastShownRef.current && incoming.length > 0) {
        restoreToastShownRef.current = true
        pushToast(
          'Projects restored from your last session. Open a terminal to continue.',
          'info',
        )
      }
      return merged
    }
    return null
  }, [pushToast])

  useEffect(() => {
    const timer = window.setTimeout(() => {
      void refreshProjects()
    }, 0)
    return () => window.clearTimeout(timer)
  }, [refreshProjects])

  const focusedProject = useMemo(() => {
    return projects.find(
      (p) => String(getProjectId(p)) === String(focusedProjectId ?? ''),
    )
  }, [projects, focusedProjectId])

  const allInstances = useMemo(() => {
    return projects.flatMap((project) =>
      getProjectInstances(project)
        .map((instance) => ({
          instance_id: String(getInstanceId(instance) ?? ''),
        }))
        .filter((instance) => instance.instance_id !== ''),
    )
  }, [projects])

  const startInstanceForProject = useCallback(async (
    projectId,
    sandboxMode,
    memLimit,
    customEnvVars,
  ) => {
    if (projectId == null) {
      pushToast('Select a project before opening a terminal', 'warn')
      return null
    }
    const project = projects.find(
      (p) => String(getProjectId(p) ?? '') === String(projectId),
    )
    const savedConfig =
      project?.config && typeof project.config === 'object'
        ? project.config
        : DEFAULT_PROJECT_CONFIG
    const resolvedSandboxMode =
      sandboxMode ??
      savedConfig.sandbox_mode ??
      project?.sandbox_mode ??
      DEFAULT_PROJECT_CONFIG.sandbox_mode
    const resolvedMemLimit =
      memLimit ?? savedConfig.mem_limit ?? DEFAULT_PROJECT_CONFIG.mem_limit
    const resolvedCustomEnvVars =
      customEnvVars ??
      (savedConfig.custom_env_vars && typeof savedConfig.custom_env_vars === 'object'
        ? savedConfig.custom_env_vars
        : DEFAULT_PROJECT_CONFIG.custom_env_vars)

    const { data, error } = await api.startInstance(
      projectId,
      resolvedSandboxMode,
      resolvedMemLimit,
      resolvedCustomEnvVars,
    )
    if (error) {
      pushToast(`Could not start terminal: ${formatApiError(error)}`, 'error')
      return null
    }
    if (data == null) {
      pushToast('Could not start terminal: empty response from server', 'error')
      return null
    }

    const newInstanceId =
      data?.instance_id ?? data?.id ?? data?.instanceId ?? null
    if (newInstanceId == null) {
      pushToast('Could not start terminal: server did not return an instance id', 'error')
      return null
    }

    const normalizedId = String(newInstanceId)
    const newInstance =
      data?.instance && typeof data.instance === 'object'
        ? { ...data.instance, instance_id: data.instance.instance_id ?? normalizedId }
        : { ...data, instance_id: normalizedId }

    setProjects((prev) =>
      prev.map((project) => {
        if (String(getProjectId(project)) !== String(projectId)) {
          return project
        }
        const existing = Array.isArray(project.instances) ? project.instances : []
        return {
          ...project,
          instances: [...existing, newInstance],
        }
      }),
    )

    setFocusedInstanceId(normalizedId)
    return normalizedId
  }, [projects, pushToast])

  const handleStartInstance = useCallback(
    async (sandboxMode) => {
      const project = projects.find(
        (p) => String(getProjectId(p) ?? '') === String(focusedProjectId ?? ''),
      )
      const savedConfig =
        project?.config && typeof project.config === 'object'
          ? project.config
          : DEFAULT_PROJECT_CONFIG
      const resolvedSandboxMode =
        sandboxMode ?? savedConfig?.sandbox_mode ?? DEFAULT_PROJECT_CONFIG.sandbox_mode
      const resolvedMemLimit =
        savedConfig?.mem_limit ?? DEFAULT_PROJECT_CONFIG.mem_limit
      const resolvedCustomEnvVars =
        savedConfig?.custom_env_vars ?? DEFAULT_PROJECT_CONFIG.custom_env_vars

      return startInstanceForProject(
        focusedProjectId,
        resolvedSandboxMode,
        resolvedMemLimit,
        resolvedCustomEnvVars,
      )
    },
    [focusedProjectId, projects, startInstanceForProject],
  )

  const handleSelectProject = useCallback(async (id) => {
    const projectId = String(id)
    const localProjectBeforeSelect = projects.find(
      (p) => String(getProjectId(p)) === projectId,
    )
    const localInstancesBeforeSelect = getProjectInstances(localProjectBeforeSelect)

    const { error } = await api.activateProject(id)
    if (error) {
      pushToast(`Could not activate project: ${formatApiError(error)}`, 'error')
      return
    }
    setFocusedProjectId(projectId)
    const refreshed = await refreshProjects()
    const selectedProject = refreshed?.find(
      (p) => String(getProjectId(p)) === projectId,
    )
    const instances = getProjectInstances(selectedProject)
    const shouldAutoStart =
      instances.length === 0 && localInstancesBeforeSelect.length === 0
    if (shouldAutoStart) {
      const cfg =
        selectedProject?.config && typeof selectedProject.config === 'object'
          ? selectedProject.config
          : DEFAULT_PROJECT_CONFIG
      await startInstanceForProject(
        projectId,
        cfg.sandbox_mode ?? selectedProject?.sandbox_mode,
        cfg.mem_limit,
        cfg.custom_env_vars ?? DEFAULT_PROJECT_CONFIG.custom_env_vars,
      )
    } else {
      const sourceInstances =
        instances.length > 0 ? instances : localInstancesBeforeSelect
      const firstId = getInstanceId(sourceInstances[0])
      setFocusedInstanceId(firstId != null ? String(firstId) : null)
    }
  }, [projects, pushToast, refreshProjects, startInstanceForProject])

  const handleAddProject = useCallback(
    async (path, sandboxMode = DEFAULT_PROJECT_CONFIG.sandbox_mode) => {
      const { data, error } = await api.createProject(path, sandboxMode)
      if (error) {
        pushToast(`Could not create project: ${formatApiError(error)}`, 'error')
        return
      }

      const newId =
        data?.id ?? data?.project_id ?? data?.projectId ?? null

      if (newId != null) {
        const pid = String(newId)
        const initialConfig = {
          ...DEFAULT_PROJECT_CONFIG,
          project_name: data?.name ?? '',
          sandbox_mode: sandboxMode,
          ...(data?.config && typeof data.config === 'object' ? data.config : {}),
        }
        setProjects((prev) => {
          const exists = prev.some((p) => String(getProjectId(p) ?? '') === pid)
          const entry = {
            ...(data && typeof data === 'object' ? data : {}),
            id: pid,
            path,
            name: data?.name,
            sandbox_mode: sandboxMode,
            config: initialConfig,
            instances: [],
          }
          return exists
            ? prev.map((p) =>
                String(getProjectId(p) ?? '') === pid
                  ? { ...p, ...entry, instances: getProjectInstances(p) }
                  : p,
              )
            : [...prev, entry]
        })
      }

      const updated = await refreshProjects()

      const resolvedId =
        newId ??
        updated?.find((p) => (p.path ?? p.project_path) === path)?.id ??
        updated?.find((p) => (p.path ?? p.project_path) === path)?.project_id

      if (resolvedId != null) {
        setProjects((prev) =>
          prev.map((project) => {
            if (String(getProjectId(project) ?? '') !== String(resolvedId)) {
              return project
            }
            const config =
              project.config && typeof project.config === 'object'
                ? { ...project.config, sandbox_mode: sandboxMode }
                : { ...DEFAULT_PROJECT_CONFIG, sandbox_mode: sandboxMode }
            return { ...project, sandbox_mode: sandboxMode, config }
          }),
        )
        await handleSelectProject(String(resolvedId))
      }
    },
    [handleSelectProject, pushToast, refreshProjects],
  )

  const handleFocusInstance = useCallback(async (id) => {
    const { error } = await api.focusInstance(id)
    if (error) {
      pushToast(`Could not focus terminal: ${formatApiError(error)}`, 'error')
      return
    }
    setFocusedInstanceId(String(id))
  }, [pushToast])

  const handleSidebarSelectInstance = useCallback(
    async (projectId, instanceId) => {
      const pid = String(projectId)
      const iid = String(instanceId)
      if (String(focusedProjectId ?? '') !== pid) {
        const { error } = await api.activateProject(pid)
        if (error) {
          pushToast(`Could not activate project: ${formatApiError(error)}`, 'error')
          return
        }
        setFocusedProjectId(pid)
      }
      await handleFocusInstance(iid)
    },
    [focusedProjectId, handleFocusInstance, pushToast],
  )

  const handleCloseProject = useCallback(
    async (projectId) => {
      const { error } = await api.unloadProject(projectId)
      if (error) {
        pushToast(`Could not unload project: ${formatApiError(error)}`, 'error')
        return
      }

      setProjects((prev) =>
        prev.filter((project) => String(getProjectId(project)) !== String(projectId)),
      )

      if (String(focusedProjectId ?? '') === String(projectId)) {
        setFocusedProjectId(null)
        setFocusedInstanceId(null)
        setHandoverOpen(false)
      }
    },
    [focusedProjectId, pushToast],
  )

  const handleStopInstance = useCallback(
    (instanceId) => {
      const targetId = String(instanceId ?? '')
      if (targetId === '') return

      setProjects((prev) => {
        let removedFromFocused = false
        let nextFocusedId = null
        const nextProjects = prev.map((project) => {
          const existing = getProjectInstances(project)
          const removeIndex = existing.findIndex(
            (inst) =>
              String(
                inst?.instance_id ?? inst?.id ?? inst?.instanceId ?? '',
              ) === targetId,
          )
          if (removeIndex < 0) return project

          const nextInstances = existing.filter((_, idx) => idx !== removeIndex)
          const projectId = String(getProjectId(project) ?? '')
          if (projectId === String(focusedProjectId ?? '')) {
            removedFromFocused = true
            if (String(focusedInstanceId ?? '') === targetId) {
              const nextFocus =
                nextInstances[removeIndex] ?? nextInstances[removeIndex - 1] ?? null
              const nextId = nextFocus
                ? String(
                    nextFocus?.instance_id ?? nextFocus?.id ?? nextFocus?.instanceId ?? '',
                  )
                : ''
              nextFocusedId = nextId !== '' ? nextId : null
            } else {
              nextFocusedId =
                focusedInstanceId != null ? String(focusedInstanceId) : null
            }
          }

          return {
            ...project,
            instances: nextInstances,
          }
        })

        if (removedFromFocused) {
          setFocusedInstanceId(nextFocusedId)
        }

        return nextProjects
      })

      void api.stopInstance(targetId).then(({ error }) => {
        if (error) {
          console.error(`Could not stop terminal ${targetId}:`, error)
        }
      })
    },
    [focusedInstanceId, focusedProjectId],
  )

  const handleExecuteHandover = useCallback(
    async (payload) => {
      const { data, error } = await api.executeHandover(payload)
      if (error) {
        pushToast(`Could not execute handover: ${formatApiError(error)}`, 'error')
        return false
      }
      const result = api.parseHandoffResult(data)
      if (!result.ok) {
        pushToast(`Could not execute handover: ${result.message}`, 'error')
        return false
      }
      pushToast(result.message ?? 'Handover executed', 'success')
      return true
    },
    [pushToast],
  )

  const handleOpenProjectSettings = useCallback((projectId) => {
    setSettingsProjectId(String(projectId))
    setIsSettingsModalOpen(true)
  }, [])

  const handleSaveProjectConfig = useCallback((projectId, config) => {
    const displayName = config?.project_name?.trim() ?? ''
    const normalizedConfig = {
      project_name: displayName,
      sandbox_mode: config?.sandbox_mode ?? DEFAULT_PROJECT_CONFIG.sandbox_mode,
      mem_limit: config?.mem_limit ?? DEFAULT_PROJECT_CONFIG.mem_limit,
      handoff_method: config?.handoff_method ?? DEFAULT_PROJECT_CONFIG.handoff_method,
      custom_env_vars:
        config?.custom_env_vars && typeof config.custom_env_vars === 'object'
          ? config.custom_env_vars
          : DEFAULT_PROJECT_CONFIG.custom_env_vars,
    }

    setProjects((prev) =>
      prev.map((project) =>
        String(getProjectId(project) ?? '') === String(projectId ?? '')
          ? {
              ...project,
              name: displayName || project.name,
              config: normalizedConfig,
            }
          : project,
      ),
    )
    pushToast('Project settings saved', 'info')
  }, [pushToast])

  return (
    <div className="relative flex min-h-0 min-w-0 flex-1 flex-col bg-[#1e1e1e] text-[#cccccc]">
      <div className="flex min-h-0 min-w-0 flex-1">
        <div
          className={`relative shrink-0 overflow-hidden border-r border-[#333333] transition-[width] duration-200 ease-in-out ${
            sidebarOpen ? 'w-[260px]' : 'w-0 border-r-0'
          }`}
        >
          <Sidebar
            projects={projects}
            focusedProjectId={focusedProjectId}
            focusedInstanceId={focusedInstanceId}
            onSelectProject={handleSelectProject}
            onSelectInstance={handleSidebarSelectInstance}
            onAddProject={handleAddProject}
            onCloseProject={handleCloseProject}
            onOpenSettings={handleOpenProjectSettings}
            onToggleSidebar={() => setSidebarOpen((open) => !open)}
          />
        </div>
        <div className="relative flex min-h-0 min-w-0 flex-1 flex-col">
          {!sidebarOpen ? (
            <button
              type="button"
              title="Show projects sidebar"
              onClick={() => setSidebarOpen(true)}
              className="absolute left-0 top-0 z-20 flex h-9 w-8 items-center justify-center border-r border-b border-[#333333] bg-[#181818] text-[#808080] hover:bg-[#2a2d2e] hover:text-[#cccccc]"
            >
              <svg width="16" height="16" viewBox="0 0 16 16" fill="currentColor" aria-hidden>
                <path d="M2 2.5h5.5v11H2V2.5zm6.5 0H14v11H8.5V2.5z" opacity="0.9" />
              </svg>
            </button>
          ) : null}
        <TabBar
          focusedProject={focusedProject}
          focusedInstanceId={focusedInstanceId}
          ptyConnections={ptyConnections}
          onStartInstance={handleStartInstance}
          onFocusInstance={handleFocusInstance}
          onStopInstance={handleStopInstance}
          onOpenHandover={() => setHandoverOpen(true)}
        />
        <div className="flex min-h-0 min-w-0 flex-1">
          {allInstances.map((instance) => (
            <TerminalView
              key={instance.instance_id}
              instanceId={instance.instance_id}
              isActive={instance.instance_id === String(focusedInstanceId ?? '')}
              onConnectionChange={handlePtyConnectionChange}
            />
          ))}
          {allInstances.length === 0 ? (
            <div className="box-border flex min-h-0 min-w-0 flex-1 items-center justify-center bg-[#1e1e1e] px-4 text-center text-sm text-[#808080]">
              Create a project and open a terminal to get started
            </div>
          ) : null}
        </div>
        </div>
      </div>
      <HandoverModal
        key={`${focusedProjectId ?? 'none'}-${handoverOpen ? 'open' : 'closed'}`}
        open={handoverOpen}
        projectId={focusedProjectId}
        instances={getProjectInstances(focusedProject)}
        ptyConnections={ptyConnections}
        defaultMethod={
          focusedProject?.config?.handoff_method ??
          DEFAULT_PROJECT_CONFIG.handoff_method
        }
        onClose={() => setHandoverOpen(false)}
        onExecute={handleExecuteHandover}
      />
      <ProjectSettings
        isOpen={isSettingsModalOpen}
        onClose={() => setIsSettingsModalOpen(false)}
        focusedProjectId={settingsProjectId}
        defaultProjectName={getProjectDisplayName(
          projects.find(
            (p) => String(getProjectId(p) ?? '') === String(settingsProjectId ?? ''),
          ),
        )}
        onSaveConfig={handleSaveProjectConfig}
      />
      <ToastStack toasts={toasts} />
    </div>
  )
}
