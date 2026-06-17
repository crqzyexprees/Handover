import { useCallback, useEffect, useState } from 'react'
import * as api from './api.js'

const POLL_MS = 5000

function formatMb(mb) {
  if (mb == null || !Number.isFinite(mb)) return '—'
  return mb >= 1024 ? `${(mb / 1024).toFixed(1)} GB` : `${mb.toFixed(0)} MB`
}

function formatPercent(value) {
  if (value == null || !Number.isFinite(value)) return '—'
  return `${value.toFixed(1)}%`
}

function RamBar({ system }) {
  if (!system) return null

  const percent = system.ram_percent ?? 0
  const suspendAt = system.suspend_threshold_percent ?? 85
  const emergencyAt = system.emergency_threshold_percent ?? 95

  const barColor =
    percent >= emergencyAt
      ? 'bg-red-500'
      : percent >= suspendAt
        ? 'bg-yellow-500'
        : 'bg-[#3794ff]'

  return (
    <section className="rounded border border-[#333333] p-3">
      <div className="mb-2 flex items-baseline justify-between text-xs">
        <span className="font-semibold uppercase tracking-wide text-[#808080]">
          System RAM
        </span>
        <span className="text-[#cccccc]">
          {formatMb(system.ram_used_mb)} / {formatMb(system.ram_total_mb)} (
          {formatPercent(percent)})
        </span>
      </div>
      <div className="relative h-3 overflow-hidden rounded bg-[#1e1e1e]">
        <div
          className={`h-full transition-all duration-300 ${barColor}`}
          style={{ width: `${Math.min(100, Math.max(0, percent))}%` }}
        />
        <div
          className="absolute top-0 h-full w-px bg-yellow-400/80"
          style={{ left: `${suspendAt}%` }}
          title={`Suspend threshold (${suspendAt}%)`}
        />
        <div
          className="absolute top-0 h-full w-px bg-red-400/80"
          style={{ left: `${emergencyAt}%` }}
          title={`Emergency threshold (${emergencyAt}%)`}
        />
      </div>
      <div className="mt-2 flex justify-between text-[10px] text-[#808080]">
        <span>Suspend at {suspendAt}%</span>
        <span>Emergency at {emergencyAt}%</span>
      </div>
    </section>
  )
}

export default function ResourceDashboardPanel({ open, projectId, onClose }) {
  const [resources, setResources] = useState(null)
  const [loading, setLoading] = useState(false)
  const [errorText, setErrorText] = useState('')

  const loadResources = useCallback(async () => {
    if (projectId == null) return
    setLoading(true)
    const { data, error } = await api.getProjectResources(projectId)
    setLoading(false)
    if (error) {
      setErrorText('Could not load resource stats.')
      return
    }
    setErrorText('')
    setResources(data)
  }, [projectId])

  useEffect(() => {
    if (!open || projectId == null) return
    const initialTimer = window.setTimeout(() => {
      void loadResources()
    }, 0)
    const timer = window.setInterval(() => {
      void loadResources()
    }, POLL_MS)
    return () => {
      window.clearTimeout(initialTimer)
      window.clearInterval(timer)
    }
  }, [open, projectId, loadResources])

  if (!open) return null

  const instances = Array.isArray(resources?.instances) ? resources.instances : []
  const projectState = resources?.project?.state ?? 'active'

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4"
      role="presentation"
      onClick={onClose}
    >
      <div
        role="dialog"
        aria-modal="true"
        className="flex max-h-[min(640px,90vh)] w-full max-w-3xl flex-col rounded-md border border-[#333333] bg-[#252526] text-[#cccccc] shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex shrink-0 items-center justify-between border-b border-[#333333] px-4 py-3">
          <div>
            <h2 className="text-sm font-semibold">Resources</h2>
            <p className="mt-0.5 text-xs text-[#808080]">
              System and terminal usage — refreshes every 5s
            </p>
          </div>
          <div className="flex items-center gap-2">
            <button
              type="button"
              onClick={() => void loadResources()}
              disabled={loading}
              className="rounded border border-[#333333] bg-[#1e1e1e] px-2 py-1 text-xs hover:bg-[#2a2d2e] disabled:opacity-40"
            >
              Refresh
            </button>
            <button
              type="button"
              onClick={onClose}
              className="rounded border border-[#333333] bg-[#1e1e1e] px-2 py-1 text-xs hover:bg-[#2a2d2e]"
            >
              Close
            </button>
          </div>
        </div>

        <div className="min-h-0 flex-1 space-y-4 overflow-y-auto p-4">
          <RamBar system={resources?.system} />

          <section className="rounded border border-[#333333] p-3">
            <div className="mb-3 flex items-center justify-between text-xs">
              <span className="font-semibold uppercase tracking-wide text-[#808080]">
                Terminals
              </span>
              <span className="text-[#808080]">
                Project {projectState} · {instances.length} instance
                {instances.length === 1 ? '' : 's'}
              </span>
            </div>

            {instances.length === 0 ? (
              <p className="text-xs text-[#808080]">No terminals for this project.</p>
            ) : (
              <div className="overflow-x-auto">
                <table className="w-full min-w-[480px] text-left text-xs">
                  <thead>
                    <tr className="border-b border-[#333333] text-[#808080]">
                      <th className="pb-2 pr-3 font-medium">Terminal</th>
                      <th className="pb-2 pr-3 font-medium">Sandbox</th>
                      <th className="pb-2 pr-3 font-medium">Connected</th>
                      <th className="pb-2 pr-3 font-medium">Memory</th>
                      <th className="pb-2 font-medium">CPU</th>
                    </tr>
                  </thead>
                  <tbody>
                    {instances.map((row) => {
                      const stats = row.stats ?? {}
                      const memUsed = stats.mem_used_mb
                      const memLimit = stats.mem_limit_mb
                      const memLabel =
                        memLimit > 0
                          ? `${formatMb(memUsed)} / ${formatMb(memLimit)}`
                          : formatMb(memUsed)
                      return (
                        <tr
                          key={row.instance_id}
                          className="border-b border-[#333333]/60 last:border-0"
                        >
                          <td className="py-2 pr-3">{row.label ?? row.instance_id}</td>
                          <td className="py-2 pr-3 capitalize">
                            {row.sandbox_mode ?? '—'}
                          </td>
                          <td className="py-2 pr-3">
                            <span
                              className={
                                row.connected
                                  ? 'text-green-400'
                                  : 'text-[#808080]'
                              }
                            >
                              {row.connected ? 'Yes' : 'No'}
                            </span>
                          </td>
                          <td className="py-2 pr-3 font-mono">{memLabel}</td>
                          <td className="py-2 font-mono">
                            {formatPercent(stats.cpu_percent)}
                          </td>
                        </tr>
                      )
                    })}
                  </tbody>
                </table>
              </div>
            )}
          </section>
        </div>

        {errorText ? (
          <p className="shrink-0 border-t border-[#333333] px-4 py-2 text-xs text-red-300">
            {errorText}
          </p>
        ) : null}
      </div>
    </div>
  )
}
