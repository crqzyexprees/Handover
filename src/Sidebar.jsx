import { useCallback, useEffect, useMemo, useState } from 'react'
import {
  getInstanceId,
  getProjectConfig,
  getProjectDisplayName,
  getProjectId,
  getProjectInstances,
  getTerminalSidebarLabel,
} from './projectUtils.js'

function IconButton({ title, onClick, disabled, children }) {
  return (
    <button
      type="button"
      title={title}
      disabled={disabled}
      onClick={onClick}
      className="flex size-7 items-center justify-center rounded text-[#808080] hover:bg-[#2a2d2e] hover:text-[#cccccc] disabled:cursor-default disabled:opacity-30 disabled:hover:bg-transparent"
    >
      {children}
    </button>
  )
}

function PanelLeftIcon() {
  return (
    <svg width="16" height="16" viewBox="0 0 16 16" fill="currentColor" aria-hidden>
      <path d="M2 2.5h5.5v11H2V2.5zm6.5 0H14v11H8.5V2.5z" opacity="0.9" />
    </svg>
  )
}

function SearchIcon() {
  return (
    <svg width="16" height="16" viewBox="0 0 16 16" fill="none" aria-hidden>
      <circle cx="7" cy="7" r="4.25" stroke="currentColor" strokeWidth="1.25" />
      <path d="M10.2 10.2L13.5 13.5" stroke="currentColor" strokeWidth="1.25" strokeLinecap="round" />
    </svg>
  )
}

function ChevronLeftIcon() {
  return (
    <svg width="16" height="16" viewBox="0 0 16 16" fill="currentColor" aria-hidden>
      <path d="M9.5 4.5L6 8l3.5 3.5" stroke="currentColor" strokeWidth="1.25" fill="none" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  )
}

function ChevronRightIcon() {
  return (
    <svg width="16" height="16" viewBox="0 0 16 16" fill="currentColor" aria-hidden>
      <path d="M6.5 4.5L10 8l-3.5 3.5" stroke="currentColor" strokeWidth="1.25" fill="none" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  )
}

function ChevronDownIcon({ className = '' }) {
  return (
    <svg width="12" height="12" viewBox="0 0 16 16" fill="currentColor" className={className} aria-hidden>
      <path d="M4 6l4 4 4-4" stroke="currentColor" strokeWidth="1.25" fill="none" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  )
}

function NewProjectIcon() {
  return (
    <svg width="16" height="16" viewBox="0 0 16 16" fill="none" aria-hidden>
      <path
        d="M8 3.5v9M3.5 8h9"
        stroke="currentColor"
        strokeWidth="1.25"
        strokeLinecap="round"
      />
    </svg>
  )
}

function SandboxChoiceModal({ path, onChoose, onCancel }) {
  return (
    <div
      className="fixed inset-0 z-[60] flex items-center justify-center bg-black/50 p-4"
      role="presentation"
      onClick={onCancel}
    >
      <div
        role="dialog"
        aria-modal="true"
        className="w-full max-w-sm rounded-md border border-[#333333] bg-[#252526] p-4 text-[#cccccc] shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <h2 className="text-sm font-semibold">New project</h2>
        <p className="mt-2 text-xs leading-relaxed text-[#808080]">
          How do you want to run terminals in this project?
        </p>
        <p
          className="mt-1 truncate text-[11px] text-[#666666]"
          title={path}
        >
          {path}
        </p>

        <div className="mt-4 flex flex-col gap-2">
          <button
            type="button"
            onClick={() => onChoose('docker')}
            className="rounded-md border border-[#333333] bg-[#1e1e1e] px-3 py-2.5 text-left text-sm text-[#cccccc] hover:bg-[#2a2d2e]"
          >
            Sandboxed (Docker)
          </button>
          <button
            type="button"
            onClick={() => onChoose('native')}
            className="rounded-md border border-[#333333] bg-[#1e1e1e] px-3 py-2.5 text-left hover:bg-[#2a2d2e]"
          >
            <span className="block text-sm text-[#cccccc]">Native (Host)</span>
            <span className="mt-1 block text-[11px] leading-snug text-[#808080]">
              Uses your local system tools. No sandbox isolation.
            </span>
          </button>
        </div>

        <div className="mt-4 flex justify-end">
          <button
            type="button"
            onClick={onCancel}
            className="rounded border border-[#333333] px-3 py-1.5 text-xs text-[#808080] hover:bg-[#2a2d2e] hover:text-[#cccccc]"
          >
            Cancel
          </button>
        </div>
      </div>
    </div>
  )
}

export default function Sidebar({
  projects,
  focusedProjectId,
  focusedInstanceId,
  onSelectProject,
  onSelectInstance,
  onAddProject,
  onCloseProject,
  onOpenSettings,
  onToggleSidebar,
}) {
  const [searchOpen, setSearchOpen] = useState(false)
  const [searchQuery, setSearchQuery] = useState('')
  const [collapsedProjects, setCollapsedProjects] = useState(() => new Set())
  const [pendingProjectPath, setPendingProjectPath] = useState(null)

  const openDirectoryPicker = useCallback(async () => {
    const path = await window.electronAPI.pickDirectory()
    if (path == null) return
    const trimmed = path.trim()
    if (!trimmed) return
    setPendingProjectPath(trimmed)
  }, [])

  const handleSandboxChoice = useCallback(
    (sandboxMode) => {
      if (pendingProjectPath == null) return
      const path = pendingProjectPath
      setPendingProjectPath(null)
      void onAddProject(path, sandboxMode)
    },
    [onAddProject, pendingProjectPath],
  )

  const cancelSandboxChoice = useCallback(() => {
    setPendingProjectPath(null)
  }, [])

  useEffect(() => {
    const onKeyDown = (e) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'n') {
        e.preventDefault()
        void openDirectoryPicker()
      }
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [openDirectoryPicker])

  const filteredProjects = useMemo(() => {
    const q = searchQuery.trim().toLowerCase()
    if (!q) return projects
    return projects.filter((project) => {
      const title = getProjectDisplayName(project).toLowerCase()
      return title.includes(q)
    })
  }, [projects, searchQuery])

  const toggleProjectCollapsed = (projectId) => {
    setCollapsedProjects((prev) => {
      const next = new Set(prev)
      if (next.has(projectId)) {
        next.delete(projectId)
      } else {
        next.add(projectId)
      }
      return next
    })
  }

  return (
    <>
    <aside className="box-border flex h-full w-[260px] flex-col bg-[#181818] text-[13px] text-[#cccccc]">
      <div className="flex shrink-0 items-center justify-between px-2 pt-2 pb-1">
        <div className="flex items-center gap-0.5">
          <IconButton title="Hide sidebar" onClick={onToggleSidebar}>
            <PanelLeftIcon />
          </IconButton>
          <IconButton
            title="Search projects"
            onClick={() => setSearchOpen((v) => !v)}
          >
            <SearchIcon />
          </IconButton>
        </div>
        <div className="flex items-center gap-0.5">
          <IconButton title="Back" disabled>
            <ChevronLeftIcon />
          </IconButton>
          <IconButton title="Forward" disabled>
            <ChevronRightIcon />
          </IconButton>
        </div>
      </div>

      {searchOpen ? (
        <div className="shrink-0 px-3 pb-2">
          <input
            type="search"
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            placeholder="Search projects..."
            className="w-full rounded border border-[#333333] bg-[#1e1e1e] px-2 py-1 text-xs text-[#cccccc] placeholder:text-[#666666] focus:border-[#3794ff] focus:outline-none"
          />
        </div>
      ) : null}

      <div className="shrink-0 px-1">
        <button
          type="button"
          onClick={() => void openDirectoryPicker()}
          className="flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-[#cccccc] hover:bg-[#2a2d2e]"
        >
          <span className="flex size-4 shrink-0 items-center justify-center text-[#808080]">
            <NewProjectIcon />
          </span>
          <span className="min-w-0 flex-1 truncate">New Project</span>
          <span className="shrink-0 text-[11px] text-[#666666]">Ctrl+N</span>
        </button>
      </div>

      <div className="mt-2 min-h-0 flex-1 overflow-y-auto px-2 pb-3">
        {filteredProjects.length === 0 ? (
          <p className="px-2 py-1 text-xs text-[#555555]">No projects yet</p>
        ) : (
          filteredProjects.map((project) => {
            const id = getProjectId(project)
            const pid = id != null ? String(id) : ''
            const title = getProjectDisplayName(project)
            const instances = getProjectInstances(project)
            const projectSelected =
              pid !== '' && pid === String(focusedProjectId ?? '')
            const isCollapsed = collapsedProjects.has(pid)
            const config = getProjectConfig(project)
            const defaultSandbox = config?.sandbox_mode ?? 'docker'
            const sandboxLabel =
              defaultSandbox === 'native' ? 'Native' : 'Docker'

            return (
              <section key={pid || title} className="mb-3">
                <div className="group flex items-center gap-0.5 px-0.5">
                  <button
                    type="button"
                    title={isCollapsed ? 'Expand project' : 'Collapse project'}
                    onClick={() => pid !== '' && toggleProjectCollapsed(pid)}
                    className="flex size-5 shrink-0 items-center justify-center rounded text-[#666666] hover:bg-[#2a2d2e] hover:text-[#cccccc]"
                  >
                    <ChevronDownIcon
                      className={`transition-transform ${isCollapsed ? '-rotate-90' : ''}`}
                    />
                  </button>
                  <button
                    type="button"
                    onClick={() => {
                      if (pid === '') return
                      if (isCollapsed) toggleProjectCollapsed(pid)
                      onSelectProject(pid)
                    }}
                    className={`min-w-0 flex-1 truncate text-left text-xs hover:text-[#cccccc] ${
                      projectSelected ? 'text-[#aaaaaa]' : 'text-[#888888]'
                    }`}
                  >
                    {title}
                  </button>
                  <span
                    className="shrink-0 rounded px-1 text-[10px] text-[#666666]"
                    title="Default sandbox for new terminals"
                  >
                    {sandboxLabel}
                  </span>
                  <button
                    type="button"
                    title="Project settings"
                    onClick={() => pid !== '' && onOpenSettings(pid)}
                    className="flex size-5 shrink-0 items-center justify-center rounded text-[11px] text-[#666666] opacity-0 hover:bg-[#2a2d2e] hover:text-[#cccccc] group-hover:opacity-100"
                  >
                    ⚙
                  </button>
                  <button
                    type="button"
                    title="Unload project"
                    onClick={() => pid !== '' && onCloseProject(pid)}
                    className="flex size-5 shrink-0 items-center justify-center rounded text-[11px] text-[#666666] opacity-0 hover:bg-[#2a2d2e] hover:text-[#cccccc] group-hover:opacity-100"
                  >
                    ×
                  </button>
                </div>

                {!isCollapsed ? (
                  instances.length === 0 ? (
                    <p className="mt-0.5 pl-6 text-xs text-[#555555]">
                      - No terminals yet
                    </p>
                  ) : (
                    <ul className="mt-0.5 space-y-0.5">
                      {instances.map((instance, index) => {
                        const instanceId = getInstanceId(instance)
                        const sid =
                          instanceId != null ? String(instanceId) : ''
                        const selected =
                          sid !== '' &&
                          sid === String(focusedInstanceId ?? '')
                        const label = getTerminalSidebarLabel(
                          project,
                          instance,
                          index,
                        )

                        return (
                          <li key={sid || `inst-${index}`}>
                            <button
                              type="button"
                              onClick={() =>
                                pid !== '' &&
                                sid !== '' &&
                                onSelectInstance(pid, sid)
                              }
                              className={`flex w-full items-center gap-2 rounded-md py-1 pr-2 pl-5 text-left text-[13px] ${
                                selected
                                  ? 'bg-[#2a2d2e] text-[#ffffff]'
                                  : 'text-[#cccccc] hover:bg-[#232323]'
                              }`}
                            >
                              <span
                                className={`size-1.5 shrink-0 rounded-full ${
                                  selected ? 'bg-[#888888]' : 'bg-[#555555]'
                                }`}
                              />
                              <span className="min-w-0 truncate">{label}</span>
                            </button>
                          </li>
                        )
                      })}
                    </ul>
                  )
                ) : null}
              </section>
            )
          })
        )}
      </div>
    </aside>
    {pendingProjectPath != null ? (
      <SandboxChoiceModal
        path={pendingProjectPath}
        onChoose={handleSandboxChoice}
        onCancel={cancelSandboxChoice}
      />
    ) : null}
    </>
  )
}
