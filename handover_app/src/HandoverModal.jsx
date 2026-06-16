import { useMemo, useState } from 'react'
import { getInstanceId } from './projectUtils.js'

const METHOD_HELP = {
  git: 'Creates a version-history checkpoint for team handoffs or review, then prompts the receiving terminal.',
  summary:
    'Default for day-to-day AI handoffs. Prompts the source terminal to write `.handover/handoffs/latest.md`, waits for it, then directs the target to read it.',
}

function connectionLabel(status) {
  if (status === 'connected') return 'connected'
  if (status === 'connecting') return 'connecting…'
  return 'disconnected'
}

function formatOptionLabel(baseLabel, status) {
  const conn = connectionLabel(status)
  return `${baseLabel} · ${conn}`
}

export default function HandoverModal({
  open,
  projectId,
  instances,
  ptyConnections = {},
  defaultMethod = 'summary',
  onClose,
  onExecute,
}) {
  const instanceOptions = useMemo(
    () =>
      (Array.isArray(instances) ? instances : [])
        .map((instance, index) => {
          const id = getInstanceId(instance)
          return id == null
            ? null
            : {
                id: String(id),
                label: `Terminal ${index + 1}`,
              }
        })
        .filter(Boolean),
    [instances],
  )

  const [fromInstanceId, setFromInstanceId] = useState('')
  const [toInstanceId, setToInstanceId] = useState('')
  const initialMethod = defaultMethod === 'summary' ? 'summary' : 'git'
  const [method, setMethod] = useState(initialMethod)
  const [taskDescription, setTaskDescription] = useState('')
  const [executing, setExecuting] = useState(false)

  if (!open) return null

  const first = instanceOptions[0]?.id ?? ''
  const second = instanceOptions.find((x) => x.id !== first)?.id ?? ''
  const effectiveFrom = fromInstanceId || first
  const effectiveTo = toInstanceId || second
  const trimmedTask = taskDescription.trim()

  const fromConnected = ptyConnections[effectiveFrom] === 'connected'
  const toConnected = ptyConnections[effectiveTo] === 'connected'
  const connectionsReady =
    method === 'summary' ? fromConnected && toConnected : toConnected

  const canExecute =
    trimmedTask !== '' &&
    effectiveFrom !== '' &&
    effectiveTo !== '' &&
    effectiveFrom !== effectiveTo &&
    instanceOptions.length >= 2 &&
    connectionsReady &&
    !executing

  const connectionWarning = (() => {
    if (instanceOptions.length < 2) {
      return 'Open a second terminal to hand over.'
    }
    if (connectionsReady) return null
    if (method === 'summary' && !fromConnected) {
      return 'Source terminal is not connected — open it and wait for ● Connected.'
    }
    if (!toConnected) {
      return 'Receiving terminal is not connected — open it and wait for ● Connected.'
    }
    return null
  })()

  const handleExecute = async () => {
    if (!canExecute) return
    setExecuting(true)
    try {
      const ok = await onExecute({
        from_instance_id: effectiveFrom,
        to_instance_id: effectiveTo,
        project_id: projectId,
        method,
        task_description: trimmedTask,
      })
      if (ok !== false) onClose()
    } finally {
      setExecuting(false)
    }
  }

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4"
      role="presentation"
      onClick={onClose}
    >
      <div
        role="dialog"
        aria-modal="true"
        className="w-full max-w-md rounded-md border border-[#333333] bg-[#252526] p-4 text-[#cccccc] shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <h2 className="text-sm font-semibold">Handover</h2>

        <div className="mt-4 space-y-3">
          <div>
            <label
              htmlFor="handover-task"
              className="block text-xs font-semibold uppercase tracking-wide text-[#808080]"
            >
              Goal for the next AI
            </label>
            <textarea
              id="handover-task"
              value={taskDescription}
              onChange={(e) => setTaskDescription(e.target.value)}
              placeholder="e.g., finish the Rust backend persistence work"
              rows={3}
              className="mt-1 w-full resize-y rounded border border-[#333333] bg-[#1e1e1e] px-2 py-1.5 text-sm text-[#cccccc] placeholder:text-[#808080] focus:border-[#3794ff] focus:outline-none"
            />
          </div>

          <div>
            <label
              htmlFor="handover-from"
              className="block text-xs font-semibold uppercase tracking-wide text-[#808080]"
            >
              From Terminal
            </label>
            <select
              id="handover-from"
              value={effectiveFrom}
              onChange={(e) => setFromInstanceId(e.target.value)}
              className="mt-1 w-full rounded border border-[#333333] bg-[#1e1e1e] px-2 py-1.5 text-sm text-[#cccccc]"
            >
              {instanceOptions.map((opt) => (
                <option key={`from-${opt.id}`} value={opt.id}>
                  {formatOptionLabel(opt.label, ptyConnections[opt.id])}
                </option>
              ))}
            </select>
          </div>

          <div>
            <label
              htmlFor="handover-to"
              className="block text-xs font-semibold uppercase tracking-wide text-[#808080]"
            >
              To Terminal
            </label>
            <select
              id="handover-to"
              value={effectiveTo}
              onChange={(e) => setToInstanceId(e.target.value)}
              className="mt-1 w-full rounded border border-[#333333] bg-[#1e1e1e] px-2 py-1.5 text-sm text-[#cccccc]"
            >
              {instanceOptions.map((opt) => (
                <option key={`to-${opt.id}`} value={opt.id}>
                  {formatOptionLabel(opt.label, ptyConnections[opt.id])}
                </option>
              ))}
            </select>
          </div>
        </div>

        <fieldset className="mt-4 rounded border border-[#333333] p-3">
          <legend className="px-1 text-xs font-semibold uppercase tracking-wide text-[#808080]">
            Handover Method
          </legend>
          <div className="mt-2 flex flex-col gap-3 text-sm">
            <label className="flex cursor-pointer items-start gap-2">
              <input
                type="radio"
                name="handoverMethod"
                value="git"
                checked={method === 'git'}
                onChange={(e) => setMethod(e.target.value)}
                className="mt-0.5"
              />
              <span>
                <span className="font-medium">Git Commit</span>
                <span className="mt-0.5 block text-xs leading-relaxed text-[#808080]">
                  {METHOD_HELP.git}
                </span>
              </span>
            </label>
            <label className="flex cursor-pointer items-start gap-2">
              <input
                type="radio"
                name="handoverMethod"
                value="summary"
                checked={method === 'summary'}
                onChange={(e) => setMethod(e.target.value)}
                className="mt-0.5"
              />
              <span>
                <span className="font-medium">Summary File</span>
                <span className="mt-0.5 block text-xs leading-relaxed text-[#808080]">
                  {METHOD_HELP.summary}
                </span>
              </span>
            </label>
          </div>
        </fieldset>

        {connectionWarning ? (
          <p className="mt-3 text-xs text-yellow-300">{connectionWarning}</p>
        ) : null}

        <div className="mt-4 flex justify-end gap-2">
          <button
            type="button"
            onClick={onClose}
            disabled={executing}
            className="rounded border border-[#333333] bg-[#1e1e1e] px-3 py-1.5 text-sm text-[#cccccc] hover:bg-[#2a2d2e] disabled:opacity-40"
          >
            Cancel
          </button>
          <button
            type="button"
            disabled={!canExecute}
            onClick={() => void handleExecute()}
            className="rounded border border-[#333333] bg-[#37373d] px-3 py-1.5 text-sm text-[#cccccc] hover:bg-[#2a2d2e] disabled:cursor-not-allowed disabled:opacity-40"
          >
            {executing ? 'Executing…' : 'Execute Handover'}
          </button>
        </div>
      </div>
    </div>
  )
}
