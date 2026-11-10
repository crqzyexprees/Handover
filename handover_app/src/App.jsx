import { useCallback, useEffect, useMemo, useReducer, useRef, useState } from 'react'
import * as api from './api.js'
import { getProjectDisplayName } from './projectUtils.js'
import {
  allInstances as listAllInstances,
  findProject,
  initialProjectState,
  projectActions,
  projectReducer,
} from './projectStore.js'
import HandoverModal from './HandoverModal.jsx'
import HandoffHistoryPanel from './HandoffHistoryPanel.jsx'
import BroadcastModal from './BroadcastModal.jsx'
import ResourceDashboardPanel from './ResourceDashboardPanel.jsx'
import ProjectSettings from './ProjectSettings.jsx'
import Sidebar from './Sidebar.jsx'
import TabBar from './TabBar.jsx'
import TerminalView from './TerminalView.jsx'
import TitleBar from './TitleBar.jsx'

/** Error text from a failed request (`request()` in api.js). */
function formatApiError(error) {
  if (error == null) return 'Unknown error'
  if (typeof error === 'string') return error
  if (typeof error !== 'object') return 'Request failed'
  if (typeof error.detail === 'string') return error.detail
  if (error.message) return String(error.message)
  if (error.httpStatus) return `Request failed (${error.httpStatus})`
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
  const [store, dispatch] = useReducer(projectReducer, initialProjectState)
  const { projects, focusedProjectId, focusedInstanceId } = store
  const [toasts, setToasts] = useState([])
  const [handoverOpen, setHandoverOpen] = useState(false)
  const [handoffHistoryOpen, setHandoffHistoryOpen] = useState(false)
  const [resourcesOpen, setResourcesOpen] = useState(false)
  const [broadcastOpen, setBroadcastOpen] = useState(false)
  const [isSettingsModalOpen, setIsSettingsModalOpen] = useState(false)
  const [settingsProjectId, setSettingsProjectId] = useState(null)
  const [sidebarOpen, setSidebarOpen] = useState(true)
  const [ptyConnections, setPtyConnections] = useState({})
  const restoreToastShownRef = useRef(false)

  const pushToast = useCallback((message, variant = 'error', durationMs = 5000) => {
    const id =
      globalThis.crypto?.randomUUID?.() ??
      `${Date.now()}-${Math.random().toString(16).slice(2)}`
    setToasts((prev) => [...prev, { id, message, variant }])
    window.setTimeout(() => {
      setToasts((prev) => prev.filter((x) => x.id !== id))
    }, durationMs)
  }, [])

  const handlePtyConnectionChange = useCallback((instanceId, status) => {
    if (!instanceId) return
    setPtyConnections((prev) => {
      if (prev[instanceId] === status) return prev
      return { ...prev, [instanceId]: status }
    })
  }, [])

  /** Reloads every Project; returns the list, or null on failure. */
  const refreshProjects = useCallback(async () => {
    const { data, error } = await api.listProjects()
    if (error) {
      pushToast(`Could not load projects: ${formatApiError(error)}`, 'error')
      return null
    }
    dispatch(projectActions.loaded(data))
    if (!restoreToastShownRef.current && data.length > 0) {
      restoreToastShownRef.current = true
      const runningTerminals = data.some((project) => project.instances.length > 0)
      pushToast(
        runningTerminals
          ? 'Projects restored; terminals that kept running are reattached.'
          : 'Projects restored from your last session. Open a terminal to continue.',
        'info',
      )
    }
    return data
  }, [pushToast])

  useEffect(() => {
    const timer = window.setTimeout(() => {
      void refreshProjects()
    }, 0)
    return () => window.clearTimeout(timer)
  }, [refreshProjects])

  const focusedProject = useMemo(
    () => findProject(store, focusedProjectId),
    [store, focusedProjectId],
  )
  const allInstances = useMemo(() => listAllInstances(store), [store])

  const startInstanceForProject = useCallback(async (projectId, sandboxMode) => {
    if (projectId == null) {
      pushToast('Select a project before opening a terminal', 'warn')
      return null
    }
    const { data, error } = await api.startInstance(projectId, sandboxMode)
    if (error) {
      pushToast(`Could not start terminal: ${formatApiError(error)}`, 'error')
      return null
    }
    dispatch(projectActions.instanceOpened(data))
    return data.id
  }, [pushToast])

  const handleStartInstance = useCallback(
    (sandboxMode) => startInstanceForProject(focusedProjectId, sandboxMode),
    [focusedProjectId, startInstanceForProject],
  )

  const handleSelectProject = useCallback(async (projectId) => {
    const { error } = await api.activateProject(projectId)
    if (error) {
      pushToast(`Could not activate project: ${formatApiError(error)}`, 'error')
      return
    }
    const refreshed = await refreshProjects()
    const selected = refreshed?.find((p) => p.id === projectId)
    if (!selected) return
    dispatch(projectActions.projectFocused(projectId))
    if (selected.instances.length === 0) {
      await startInstanceForProject(projectId)
    }
  }, [pushToast, refreshProjects, startInstanceForProject])

  const handleAddProject = useCallback(
    async (path, sandboxMode) => {
      const { data, error } = await api.createProject(path, sandboxMode)
      if (error) {
        pushToast(`Could not create project: ${formatApiError(error)}`, 'error')
        return
      }
      dispatch(projectActions.projectAdded(data))
      await handleSelectProject(data.id)
    },
    [handleSelectProject, pushToast],
  )

  const handleFocusInstance = useCallback(async (id) => {
    const { error } = await api.focusInstance(id)
    if (error) {
      pushToast(`Could not focus terminal: ${formatApiError(error)}`, 'error')
      return
    }
    dispatch(projectActions.instanceFocused(id))
  }, [pushToast])

  const handleSidebarSelectInstance = useCallback(
    async (projectId, instanceId) => {
      if (focusedProjectId !== projectId) {
        const { error } = await api.activateProject(projectId)
        if (error) {
          pushToast(`Could not activate project: ${formatApiError(error)}`, 'error')
          return
        }
      }
      await handleFocusInstance(instanceId)
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
      if (focusedProjectId === projectId) {
        setHandoverOpen(false)
      }
      dispatch(projectActions.projectClosed(projectId))
    },
    [focusedProjectId, pushToast],
  )

  const handleStopInstance = useCallback((instanceId) => {
    if (!instanceId) return
    dispatch(projectActions.instanceClosed(instanceId))
    void api.stopInstance(instanceId).then(({ error }) => {
      if (error) {
        console.error(`Could not stop terminal ${instanceId}:`, error)
      }
    })
  }, [])

  const handleExecuteHandover = useCallback(
    async (payload) => {
      const result = await api.executeHandover(payload)
      if (!result.ok) {
        pushToast(result.message, 'error', 9000)
        return false
      }
      const isSummarySuccess = result.kind === 'summary_written'
      pushToast(result.message ?? 'Handover complete', 'success', isSummarySuccess ? 7000 : 5000)
      return true
    },
    [pushToast],
  )

  const handleOpenProjectSettings = useCallback((projectId) => {
    setSettingsProjectId(projectId)
    setIsSettingsModalOpen(true)
  }, [])

  /** `config` is what the backend saved (and will use). */
  const handleSaveProjectConfig = useCallback((projectId, config) => {
    dispatch(projectActions.configSaved(projectId, config))
    pushToast('Project settings saved', 'info')
  }, [pushToast])

  const settingsProject = findProject(store, settingsProjectId)

  return (
    <div className="relative flex min-h-0 min-w-0 flex-1 flex-col bg-[#1e1e1e] text-[#cccccc]">
      <TitleBar />
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
          <div className="flex shrink-0 items-stretch">
            {!sidebarOpen ? (
              <button
                type="button"
                title="Show projects sidebar"
                onClick={() => setSidebarOpen(true)}
                className="flex h-10 w-8 shrink-0 items-center justify-center border-r border-b border-[#333333] bg-[#252526] text-[#808080] hover:bg-[#2a2d2e] hover:text-[#cccccc]"
              >
                <svg width="16" height="16" viewBox="0 0 16 16" fill="currentColor" aria-hidden>
                  <path d="M2 2.5h5.5v11H2V2.5zm6.5 0H14v11H8.5V2.5z" opacity="0.9" />
                </svg>
              </button>
            ) : null}
            <div className="min-w-0 flex-1">
        <TabBar
          focusedProject={focusedProject}
          focusedInstanceId={focusedInstanceId}
          ptyConnections={ptyConnections}
          onStartInstance={handleStartInstance}
          onFocusInstance={handleFocusInstance}
          onStopInstance={handleStopInstance}
          onOpenHandover={() => setHandoverOpen(true)}
          onOpenHandoffHistory={() => setHandoffHistoryOpen(true)}
          onOpenResources={() => setResourcesOpen(true)}
          onOpenBroadcast={() => setBroadcastOpen(true)}
        />
            </div>
          </div>
        <div className="relative flex min-h-0 min-w-0 flex-1">
          {allInstances.length === 0 ? (
            <div className="box-border flex min-h-0 min-w-0 flex-1 items-center justify-center bg-[#1e1e1e] px-4 text-center text-sm text-[#808080]">
              Create a project and open a terminal to get started
            </div>
          ) : focusedInstanceId == null || focusedInstanceId === '' ? (
            <div className="box-border flex min-h-0 min-w-0 flex-1 items-center justify-center bg-[#1e1e1e] px-4 text-center text-sm text-[#808080]">
              Select a terminal tab to continue
            </div>
          ) : null}
          {/* Keep every instance mounted (native + docker) so tab switches
              do not dispose xterm or close the PTY WebSocket. */}
          {allInstances.map((instance) => (
            <TerminalView
              key={instance.id}
              instanceId={instance.id}
              isActive={instance.id === focusedInstanceId}
              onConnectionChange={handlePtyConnectionChange}
            />
          ))}
        </div>
        </div>
      </div>
      <HandoverModal
        key={`${focusedProjectId ?? 'none'}-${handoverOpen ? 'open' : 'closed'}`}
        open={handoverOpen}
        projectId={focusedProjectId}
        instances={focusedProject?.instances ?? []}
        ptyConnections={ptyConnections}
        defaultMethod={focusedProject?.config.handoff_method}
        onClose={() => setHandoverOpen(false)}
        onExecute={handleExecuteHandover}
      />
      <HandoffHistoryPanel
        open={handoffHistoryOpen}
        projectId={focusedProjectId}
        projectName={getProjectDisplayName(focusedProject)}
        onClose={() => setHandoffHistoryOpen(false)}
      />
      <ResourceDashboardPanel
        open={resourcesOpen}
        projectId={focusedProjectId}
        onClose={() => setResourcesOpen(false)}
      />
      <BroadcastModal
        key={`broadcast-${focusedProjectId ?? 'none'}-${broadcastOpen ? 'open' : 'closed'}`}
        open={broadcastOpen}
        projectId={focusedProjectId}
        instances={focusedProject?.instances ?? []}
        ptyConnections={ptyConnections}
        onClose={() => setBroadcastOpen(false)}
      />
      {isSettingsModalOpen && settingsProject ? (
        <ProjectSettings
          key={settingsProject.id}
          isOpen
          onClose={() => setIsSettingsModalOpen(false)}
          focusedProjectId={settingsProject.id}
          initialConfig={settingsProject.config}
          onSaveConfig={handleSaveProjectConfig}
        />
      ) : null}
      <ToastStack toasts={toasts} />
    </div>
  )
}
