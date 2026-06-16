import { useEffect, useMemo, useState } from 'react'
import * as api from './api.js'

const DEFAULT_CONFIG = {
  project_name: '',
  sandbox_mode: 'docker',
  mem_limit: '2g',
  handoff_method: 'summary',
}

const MEMORY_OPTIONS = ['512m', '1g', '2g', '4g']

const inputClassName =
  'min-w-0 flex-1 rounded border border-[#333333] bg-[#1e1e1e] px-2 py-2 text-sm text-[#cccccc] placeholder:text-[#666666] focus:border-[#3794ff] focus:outline-none'

function newEnvRow(key = '', value = '') {
  return {
    id:
      globalThis.crypto?.randomUUID?.() ??
      `env-${Date.now()}-${Math.random().toString(16).slice(2)}`,
    key,
    value,
  }
}

function defaultCustomEnvVars() {
  return [newEnvRow('', '')]
}

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

function dictToCustomEnvVars(dict) {
  if (!dict || typeof dict !== 'object') {
    return defaultCustomEnvVars()
  }
  const entries = Object.entries(dict)
  if (entries.length === 0) {
    return defaultCustomEnvVars()
  }
  return entries.map(([key, value]) => newEnvRow(key, String(value ?? '')))
}

function customEnvVarsToDict(rows) {
  const out = {}
  for (const row of rows) {
    const k = row.key?.trim()
    if (!k) continue
    out[k] = row.value ?? ''
  }
  return out
}

function isSensitiveEnvKey(key) {
  const upper = String(key ?? '').toUpperCase()
  return upper.includes('KEY') || upper.includes('SECRET')
}

export default function ProjectSettings({
  isOpen,
  onClose,
  focusedProjectId,
  defaultProjectName = '',
  onSaveConfig,
}) {
  const [config, setConfig] = useState(DEFAULT_CONFIG)
  const [customEnvVars, setCustomEnvVars] = useState(defaultCustomEnvVars)
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
        if (error?.httpStatus === 404 || error?.status === 404) {
          setConfig({ ...DEFAULT_CONFIG, project_name: fallbackName })
        } else {
          setErrorText('Could not load project settings. Using defaults.')
          setConfig({ ...DEFAULT_CONFIG, project_name: fallbackName })
        }
        setCustomEnvVars(defaultCustomEnvVars())
      } else {
        setConfig({
          project_name: data?.project_name?.trim() || fallbackName,
          sandbox_mode: data?.sandbox_mode ?? DEFAULT_CONFIG.sandbox_mode,
          mem_limit: data?.mem_limit ?? DEFAULT_CONFIG.mem_limit,
          handoff_method: data?.handoff_method ?? DEFAULT_CONFIG.handoff_method,
        })
        setCustomEnvVars(dictToCustomEnvVars(data?.custom_env_vars))
      }
    })

    return () => {
      ignore = true
    }
  }, [defaultProjectName, focusedProjectId, isOpen])

  if (!isOpen) return null

  const addCustomEnvVar = () => {
    setCustomEnvVars((prev) => [...prev, newEnvRow('', '')])
  }

  const updateCustomEnvVar = (id, field, nextValue) => {
    setCustomEnvVars((prev) =>
      prev.map((row) => (row.id === id ? { ...row, [field]: nextValue } : row)),
    )
  }

  const removeCustomEnvVar = (id) => {
    setCustomEnvVars((prev) => {
      const next = prev.filter((row) => row.id !== id)
      return next.length > 0 ? next : defaultCustomEnvVars()
    })
  }

  const handleSave = async () => {
    if (focusedProjectId == null) return
    if (!config.project_name?.trim()) {
      setErrorText('Project name is required.')
      return
    }
    setIsSaving(true)
    setErrorText('')
    const custom_env_vars = customEnvVarsToDict(customEnvVars)
    const payload = {
      ...config,
      project_name: config.project_name.trim(),
      custom_env_vars,
    }
    const { error } = await api.saveProjectConfig(focusedProjectId, payload)
    setIsSaving(false)
    if (error) {
      setErrorText('Could not save project settings.')
      return
    }
    if (typeof onSaveConfig === 'function') {
      onSaveConfig(focusedProjectId, payload)
    }
    window.setTimeout(() => {
      onClose()
    }, 250)
  }

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 px-4">
      <div className="flex max-h-[92vh] w-full max-w-xl flex-col rounded-md border border-[#333333] bg-[#252526] p-5 text-[#cccccc] shadow-2xl">
        <div className="mb-4 flex shrink-0 items-center justify-between">
          <h2 className="text-base font-semibold text-white">Project Settings</h2>
          <button
            type="button"
            onClick={onClose}
            className="rounded border border-[#333333] px-2 py-0.5 text-xs text-[#cccccc] hover:bg-[#2a2d2e]"
          >
            Close
          </button>
        </div>

        <div className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto pr-1">
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
            <div className="mb-2">
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
              <p className="mt-1 pl-5 text-xs text-[#808080]">
                Default for AI handoffs. Source AI writes `.handover/handoffs/latest.md`;
                backend waits up to 60s for the file, then prompts the target. Both
                terminals must be connected.
              </p>
            </div>
            <div>
              <label className="flex items-center gap-2 text-sm">
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
              <p className="mt-1 pl-5 text-xs text-[#808080]">
                Team checkpoint for switching teams on the same repo. Commits changes,
                then prompts the receiving terminal only.
              </p>
            </div>
          </section>

          <section className="rounded-lg border-2 border-[#3794ff]/40 bg-[#1e1e1e] p-4">
            <h3 className="mb-1 text-lg font-bold text-white">
              Custom API Endpoints &amp; Keys
            </h3>
            <p className="mb-4 text-xs text-[#808080]">
              Passed into Docker containers as environment variables when you start a
              terminal.
            </p>

            <ul className="space-y-3">
              {customEnvVars.map((row) => (
                <li key={row.id} className="flex items-center gap-2">
                  <input
                    type="text"
                    value={row.key}
                    onChange={(e) =>
                      updateCustomEnvVar(row.id, 'key', e.target.value)
                    }
                    placeholder="ENV_KEY (e.g., OPENAI_BASE_URL)"
                    className={inputClassName}
                  />
                  <input
                    type={isSensitiveEnvKey(row.key) ? 'password' : 'text'}
                    value={row.value}
                    onChange={(e) =>
                      updateCustomEnvVar(row.id, 'value', e.target.value)
                    }
                    placeholder="Value (e.g., https://openrouter.ai/api/v1)"
                    className={inputClassName}
                  />
                  <button
                    type="button"
                    title="Remove variable"
                    onClick={() => removeCustomEnvVar(row.id)}
                    className="flex size-9 shrink-0 items-center justify-center rounded border border-red-900/60 bg-red-950/40 text-lg font-bold text-red-400 hover:bg-red-900/50 hover:text-red-300"
                  >
                    ×
                  </button>
                </li>
              ))}
            </ul>

            <button
              type="button"
              onClick={addCustomEnvVar}
              className="mt-4 w-full rounded-md border border-[#3794ff] bg-[#252526] px-4 py-3 text-sm font-semibold text-white hover:bg-[#2a2d2e]"
            >
              + Add Environment Variable
            </button>
          </section>
        </div>

        {errorText ? <p className="mt-3 shrink-0 text-xs text-red-300">{errorText}</p> : null}
        <div className="mt-4 flex shrink-0 justify-end">
          <button
            type="button"
            disabled={isSaving}
            onClick={() => void handleSave()}
            className="rounded border border-[#333333] bg-[#37373d] px-4 py-2 text-sm font-medium text-white hover:bg-[#2a2d2e] disabled:opacity-50"
          >
            {isSaving ? 'Saving...' : 'Save'}
          </button>
        </div>
      </div>
    </div>
  )
}
