import { useEffect, useMemo, useState } from 'react'
import * as api from './api.js'

const DEFAULT_CONFIG = {
  project_name: '',
  sandbox_mode: 'docker',
  mem_limit: '2g',
  handoff_method: 'git',
}

const MEMORY_OPTIONS = ['512m', '1g', '2g', '4g']

function toGigabytes(memLimit) {
  const value = String(memLimit ?? '').trim().toLowerCase()
  if (value.endsWith('g')) {
    const parsed = Number.parseFloat(value.slice(0, -1))
    return Number.isFinite(parsed) ? parsed : 0
  }
  if (value.endsWith('m')) {
    const parsed = Number.parseFloat(value.slice(0, -1))
    return Number.isFinite(parsed) ? parsed / 1024 : 0
  }
  return 0
}

export default function ProjectSettings({
  isOpen,
  onClose,
  focusedProjectId,
  defaultProjectName = '',
  onSaveConfig,
}) {
  const [config, setConfig] = useState(DEFAULT_CONFIG)
  const [isSaving, setIsSaving] = useState(false)
  const [errorText, setErrorText] = useState('')

  const systemRamGb =
    typeof navigator !== 'undefined' && Number.isFinite(navigator.deviceMemory)
      ? Number(navigator.deviceMemory)
      : 16

  const showMemoryWarning = useMemo(() => {
    if (config.sandbox_mode !== 'docker') return false
    const selectedGb = toGigabytes(config.mem_limit)
    return selectedGb > systemRamGb / 2
  }, [config.mem_limit, config.sandbox_mode, systemRamGb])

  useEffect(() => {
    if (!isOpen || focusedProjectId == null) return

    let ignore = false

    void api.getProjectConfig(focusedProjectId).then(({ data, error }) => {
      if (ignore) return

      const fallbackName = defaultProjectName.trim() || ''
      if (error) {
        if (error?.status === 404) {
          setConfig({ ...DEFAULT_CONFIG, project_name: fallbackName })
        } else {
          setErrorText('Could not load project settings. Using defaults.')
          setConfig({ ...DEFAULT_CONFIG, project_name: fallbackName })
        }
      } else {
        setConfig({
          project_name: data?.project_name?.trim() || fallbackName,
          sandbox_mode: data?.sandbox_mode ?? DEFAULT_CONFIG.sandbox_mode,
          mem_limit: data?.mem_limit ?? DEFAULT_CONFIG.mem_limit,
          handoff_method: data?.handoff_method ?? DEFAULT_CONFIG.handoff_method,
        })
      }
    })

    return () => {
      ignore = true
    }
  }, [defaultProjectName, focusedProjectId, isOpen])

  if (!isOpen) return null

  const handleSave = async () => {
    if (focusedProjectId == null) return
    if (!config.project_name?.trim()) {
      setErrorText('Project name is required.')
      return
    }
    setIsSaving(true)
    setErrorText('')
    const { error } = await api.saveProjectConfig(focusedProjectId, {
      ...config,
      project_name: config.project_name.trim(),
    })
    setIsSaving(false)
    if (error) {
      setErrorText('Could not save project settings.')
      return
    }
    if (typeof onSaveConfig === 'function') {
      onSaveConfig(focusedProjectId, config)
    }
    window.setTimeout(() => {
      onClose()
    }, 250)
  }

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 px-4">
      <div className="w-full max-w-md rounded-md border border-[#333333] bg-[#252526] p-4 text-[#cccccc] shadow-2xl">
        <div className="mb-4 flex items-center justify-between">
          <h2 className="text-sm font-semibold">Project Settings</h2>
          <button
            type="button"
            onClick={onClose}
            className="rounded border border-[#333333] px-2 py-0.5 text-xs text-[#cccccc] hover:bg-[#2a2d2e]"
          >
            Close
          </button>
        </div>

        <div className="flex flex-col gap-4">
          <section className="rounded border border-[#333333] p-3">
            <label
              htmlFor="project-name"
              className="mb-2 block text-xs font-semibold uppercase tracking-wide text-[#808080]"
            >
              Project Name
            </label>
            <input
              id="project-name"
              type="text"
              value={config.project_name}
              onChange={(e) =>
                setConfig((prev) => ({ ...prev, project_name: e.target.value }))
              }
              placeholder="e.g. Todo API"
              className="w-full rounded border border-[#333333] bg-[#1e1e1e] px-2 py-1.5 text-sm text-[#cccccc] placeholder:text-[#666666] focus:border-[#3794ff] focus:outline-none"
            />
            <p className="mt-1 text-[11px] text-[#666666]">
              Shown in the sidebar.
            </p>
          </section>

          <section className="rounded border border-[#333333] p-3">
            <div className="mb-2 text-xs font-semibold uppercase tracking-wide text-[#808080]">
              Default Sandbox Mode
            </div>
            <label className="mb-2 flex items-center gap-2 text-sm">
              <input
                type="radio"
                name="sandbox_mode"
                checked={config.sandbox_mode === 'docker'}
                onChange={() =>
                  setConfig((prev) => ({ ...prev, sandbox_mode: 'docker' }))
                }
              />
              Docker (Sandboxed)
            </label>
            <label className="flex items-center gap-2 text-sm">
              <input
                type="radio"
                name="sandbox_mode"
                checked={config.sandbox_mode === 'native'}
                onChange={() =>
                  setConfig((prev) => ({ ...prev, sandbox_mode: 'native' }))
                }
              />
              Native (Host)
            </label>
          </section>

          {config.sandbox_mode === 'docker' ? (
            <section className="rounded border border-[#333333] p-3">
              <div className="mb-2 text-xs font-semibold uppercase tracking-wide text-[#808080]">
                Memory Limit
              </div>
              <select
                value={config.mem_limit}
                onChange={(e) =>
                  setConfig((prev) => ({ ...prev, mem_limit: e.target.value }))
                }
                className="w-full rounded border border-[#333333] bg-[#1e1e1e] px-2 py-1 text-sm text-[#cccccc]"
              >
                {MEMORY_OPTIONS.map((option) => (
                  <option key={option} value={option}>
                    {option}
                  </option>
                ))}
              </select>
              {showMemoryWarning ? (
                <div className="mt-2 rounded border border-yellow-600/70 bg-yellow-500/15 px-2 py-1 text-xs text-yellow-300">
                  Warning: selected memory is above half of system RAM (~{systemRamGb}g).
                </div>
              ) : null}
            </section>
          ) : null}

          <section className="rounded border border-[#333333] p-3">
            <div className="mb-2 text-xs font-semibold uppercase tracking-wide text-[#808080]">
              Default Handover Method
            </div>
            <label className="mb-2 flex items-center gap-2 text-sm">
              <input
                type="radio"
                name="handoff_method"
                checked={config.handoff_method === 'git'}
                onChange={() =>
                  setConfig((prev) => ({ ...prev, handoff_method: 'git' }))
                }
              />
              Git Commit
            </label>
            <label className="flex items-center gap-2 text-sm">
              <input
                type="radio"
                name="handoff_method"
                checked={config.handoff_method === 'summary'}
                onChange={() =>
                  setConfig((prev) => ({ ...prev, handoff_method: 'summary' }))
                }
              />
              Summary File
            </label>
          </section>
        </div>

        {errorText ? <p className="mt-3 text-xs text-red-300">{errorText}</p> : null}
        <div className="mt-4 flex justify-end">
          <button
            type="button"
            disabled={isSaving}
            onClick={() => void handleSave()}
            className="rounded border border-[#333333] bg-[#1e1e1e] px-3 py-1.5 text-sm text-[#cccccc] hover:bg-[#2a2d2e] disabled:opacity-50"
          >
            {isSaving ? 'Saving...' : 'Save'}
          </button>
        </div>
      </div>
    </div>
  )
}
