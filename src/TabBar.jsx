import { useState } from 'react'
import { getInstanceId } from './projectUtils.js'

export default function TabBar({
  focusedProject,
  focusedInstanceId,
  onStartInstance,
  onFocusInstance,
  onStopInstance,
  onOpenHandover,
}) {
  const [showCreateMenu, setShowCreateMenu] = useState(false)

  const instances = Array.isArray(focusedProject?.instances)
    ? focusedProject.instances
    : Array.isArray(focusedProject?.instance_list)
      ? focusedProject.instance_list
      : []

  const handleStart = async (sandboxMode) => {
    if (!focusedProject) return
    await onStartInstance(sandboxMode)
    setShowCreateMenu(false)
  }

  return (
    <header className="relative box-border flex h-10 min-h-10 shrink-0 items-stretch gap-1 overflow-visible border-b border-[#333333] bg-[#252526] px-1 text-xs text-[#cccccc]">
      <div className="flex min-w-0 flex-1 items-stretch gap-1 overflow-x-auto">
        {instances.map((instance, index) => {
          const id = getInstanceId(instance)
          const sid = id != null ? String(id) : ''
          const selected = sid !== '' && sid === String(focusedInstanceId ?? '')

          return (
            <div
              key={sid || JSON.stringify(instance)}
              className={`group flex shrink-0 items-stretch overflow-hidden rounded-sm border border-[#333333] ${
                selected ? 'bg-[#37373d]' : 'bg-[#252526] hover:bg-[#2a2d2e]'
              }`}
            >
              <button
                type="button"
                onClick={() => sid !== '' && onFocusInstance(sid)}
                className={`flex max-w-[160px] min-w-0 items-center gap-1 border-b px-2 py-1 text-left font-medium ${
                  selected
                    ? 'border-[#3794ff] text-[#cccccc]'
                    : 'border-transparent text-[#cccccc]'
                }`}
              >
                <span className="truncate">{`Terminal ${index + 1}`}</span>
              </button>
              <button
                type="button"
                title="Close terminal"
                onClick={() => sid !== '' && onStopInstance(sid)}
                className="flex w-6 shrink-0 items-center justify-center text-[11px] text-[#808080] opacity-0 transition-opacity hover:bg-[#2a2d2e] hover:text-[#cccccc] group-hover:opacity-100"
              >
                ×
              </button>
            </div>
          )
        })}
      </div>

      <div className="relative my-0.5 flex shrink-0 items-center gap-1 self-center">
        {instances.length >= 2 ? (
          <button
            type="button"
            title="Execute handover"
            onClick={onOpenHandover}
            className="rounded-sm border border-[#333333] bg-[#1e1e1e] px-2 py-1 text-[11px] font-medium text-[#cccccc] hover:bg-[#2a2d2e]"
          >
            ↔ Handover
          </button>
        ) : null}
        <button
          type="button"
          title="Add instance"
          disabled={!focusedProject}
          onClick={() => setShowCreateMenu((v) => !v)}
          className="flex w-8 items-center justify-center rounded-sm border border-[#333333] bg-[#1e1e1e] text-base leading-none text-[#cccccc] hover:bg-[#2a2d2e] disabled:cursor-not-allowed disabled:opacity-40"
        >
          +
        </button>
        {showCreateMenu ? (
          <div className="absolute right-0 top-full z-50 mt-1 min-w-[190px] rounded-sm border border-[#333333] bg-[#1e1e1e] p-2 shadow-xl">
            <div className="mb-2 text-[11px] font-medium text-[#808080]">Open as:</div>
            <div className="flex flex-col gap-1">
              <button
                type="button"
                onClick={() => void handleStart('docker')}
                className="rounded-sm border border-[#333333] px-2 py-1 text-left text-xs text-[#cccccc] hover:bg-[#2a2d2e]"
              >
                Sandboxed (Docker)
              </button>
              <button
                type="button"
                onClick={() => void handleStart('native')}
                className="rounded-sm border border-[#333333] px-2 py-1 text-left text-xs text-[#cccccc] hover:bg-[#2a2d2e]"
              >
                Native (Host)
              </button>
            </div>
          </div>
        ) : null}
      </div>
    </header>
  )
}
