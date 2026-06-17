import { useMemo, useState } from 'react'
import { getInstanceId } from './projectUtils.js'
import * as api from './api.js'

export default function BroadcastModal({
  open,
  projectId,
  instances,
  ptyConnections = {},
  onClose,
}) {
  const [prompt, setPrompt] = useState('')
  const [sending, setSending] = useState(false)
  const [result, setResult] = useState(null)
  const [errorText, setErrorText] = useState('')

  const instanceList = useMemo(
    () =>
      (Array.isArray(instances) ? instances : [])
        .map((instance, index) => {
          const id = getInstanceId(instance)
          return id == null
            ? null
            : { id: String(id), label: `Terminal ${index + 1}` }
        })
        .filter(Boolean),
    [instances],
  )

  const disconnectedCount = useMemo(
    () =>
      instanceList.filter(
        (inst) => ptyConnections[inst.id] !== 'connected',
      ).length,
    [instanceList, ptyConnections],
  )

  if (!open) return null

  const trimmed = prompt.trim()
  const canSend = trimmed !== '' && projectId != null && !sending

  const handleSend = async () => {
    if (!canSend) return
    setSending(true)
    setErrorText('')
    setResult(null)
    const { data, error } = await api.broadcastPrompt(projectId, trimmed)
    setSending(false)
    if (error) {
      setErrorText(
        typeof error.detail === 'string'
          ? error.detail
          : 'Could not broadcast prompt.',
      )
      return
    }
    setResult({
      sent: data?.sent ?? 0,
      total: data?.total ?? instanceList.length,
    })
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
        <h2 className="text-sm font-semibold">Broadcast to All Terminals</h2>
        <p className="mt-1 text-xs text-[#808080]">
          Sends the same prompt to every terminal in this project.
        </p>

        <textarea
          value={prompt}
          onChange={(e) => setPrompt(e.target.value)}
          placeholder="e.g., run the test suite and report failures"
          rows={4}
          className="mt-4 w-full resize-y rounded border border-[#333333] bg-[#1e1e1e] px-2 py-1.5 text-sm text-[#cccccc] placeholder:text-[#808080] focus:border-[#3794ff] focus:outline-none"
        />

        {disconnectedCount > 0 ? (
          <p className="mt-3 text-xs text-yellow-300">
            {disconnectedCount} of {instanceList.length} terminal
            {instanceList.length === 1 ? '' : 's'} not connected — prompt may not
            reach disconnected sessions.
          </p>
        ) : instanceList.length > 0 ? (
          <p className="mt-3 text-xs text-[#808080]">
            All {instanceList.length} terminal
            {instanceList.length === 1 ? '' : 's'} connected.
          </p>
        ) : null}

        {result ? (
          <p className="mt-3 text-xs text-green-300">
            Sent to {result.sent} of {result.total} terminal
            {result.total === 1 ? '' : 's'}.
            {result.sent < result.total
              ? ' Some terminals had no active PTY session.'
              : ''}
          </p>
        ) : null}

        {errorText ? (
          <p className="mt-3 text-xs text-red-300">{errorText}</p>
        ) : null}

        <div className="mt-4 flex justify-end gap-2">
          <button
            type="button"
            onClick={onClose}
            disabled={sending}
            className="rounded border border-[#333333] bg-[#1e1e1e] px-3 py-1.5 text-sm text-[#cccccc] hover:bg-[#2a2d2e] disabled:opacity-40"
          >
            {result ? 'Close' : 'Cancel'}
          </button>
          <button
            type="button"
            disabled={!canSend}
            onClick={() => void handleSend()}
            className="rounded border border-[#333333] bg-[#37373d] px-3 py-1.5 text-sm text-[#cccccc] hover:bg-[#2a2d2e] disabled:cursor-not-allowed disabled:opacity-40"
          >
            {sending ? 'Sending…' : 'Send to All'}
          </button>
        </div>
      </div>
    </div>
  )
}
