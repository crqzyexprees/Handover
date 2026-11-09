import { useEffect, useRef, useState } from 'react'
import { getInstanceStats } from './api'
import './terminalSessionDeps.js'
import { TerminalSession } from './terminalSession.js'
import { attachTerminalShortcuts } from './terminalShortcuts.js'

const STATS_POLL_MS = 5000

const STATUS_LABEL = {
  connecting: 'Connecting…',
  connected: 'Connected',
  disconnected: 'Disconnected',
}

const STATUS_COLOR = {
  connecting: 'text-yellow-400',
  connected: 'text-green-400',
  disconnected: 'text-red-400',
}

/**
 * Thin view over a TerminalSession (one per instanceId, native or docker).
 * When inactive, stay mounted + connected — only hide visually so scrollback survives tab switches.
 */
export default function TerminalView({ instanceId, isActive = true, onConnectionChange }) {
  const containerRef = useRef(null)
  const sessionRef = useRef(null)
  const isActiveRef = useRef(isActive)
  const onConnectionChangeRef = useRef(onConnectionChange)
  const [statsState, setStatsState] = useState(null)
  const [connectionStatus, setConnectionStatus] = useState('disconnected')

  useEffect(() => {
    onConnectionChangeRef.current = onConnectionChange
  }, [onConnectionChange])

  useEffect(() => {
    isActiveRef.current = isActive
  }, [isActive])

  // One session per instanceId. Tab switches and reconnects must NOT remount it.
  useEffect(() => {
    if (instanceId == null || instanceId === '') return
    const container = containerRef.current
    if (!container) return

    const session = new TerminalSession({ instanceId, container })
    sessionRef.current = session

    const report = (status) => {
      setConnectionStatus(status)
      onConnectionChangeRef.current?.(instanceId, status)
    }
    report(session.status)
    const unsubscribe = session.onStatus(report)
    const detachShortcuts = attachTerminalShortcuts(session, container)

    const focusTimer = window.setTimeout(() => {
      if (isActiveRef.current) session.focus()
    }, 50)

    return () => {
      window.clearTimeout(focusTimer)
      detachShortcuts()
      unsubscribe()
      session.dispose()
      onConnectionChangeRef.current?.(instanceId, 'disconnected')
      sessionRef.current = null
    }
  }, [instanceId])

  // When tab becomes active: refit + focus (no reconnect / no PTY respawn).
  useEffect(() => {
    if (!isActive) return
    const timer = window.setTimeout(() => sessionRef.current?.focus(), 0)
    return () => window.clearTimeout(timer)
  }, [isActive, instanceId])

  useEffect(() => {
    if (instanceId == null || instanceId === '' || !isActive) return

    let cancelled = false

    const fetchStats = async () => {
      const { data, error } = await getInstanceStats(instanceId)
      if (!cancelled && data && !error) {
        setStatsState({ instanceId, data })
      }
    }

    fetchStats()
    const intervalId = setInterval(fetchStats, STATS_POLL_MS)

    return () => {
      cancelled = true
      clearInterval(intervalId)
    }
  }, [instanceId, isActive])

  const stats = statsState?.instanceId === instanceId ? statsState.data : null

  const memOverLimit =
    stats != null &&
    stats.mem_limit_mb > 0 &&
    stats.mem_used_mb / stats.mem_limit_mb > 0.8

  // Keep inactive panes sized (absolute fill) instead of display:none so
  // xterm's canvas stays valid; WS/PTY stay open for native and docker alike.
  return (
    <div
      className={
        isActive
          ? 'relative z-10 flex min-h-0 min-w-0 flex-1 flex-col'
          : 'pointer-events-none invisible absolute inset-0 z-0 flex min-h-0 min-w-0 flex-col'
      }
      aria-hidden={!isActive}
    >
      <div className="box-border flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden bg-[#1e1e1e]">
        <div
          className="relative min-h-0 min-w-0 flex-1"
          onClick={() => {
            if (!isActive) return
            sessionRef.current?.focus()
          }}
        >
          <div ref={containerRef} className="h-full min-h-0 min-w-0" />
          {isActive ? (
            <div className="pointer-events-none absolute right-2 bottom-2 z-10">
              <div className="flex items-center gap-1.5 rounded bg-[#1e1e1e]/80 px-2 py-1 text-[10px]">
                <span className={STATUS_COLOR[connectionStatus] ?? STATUS_COLOR.disconnected}>
                  ● {STATUS_LABEL[connectionStatus] ?? connectionStatus}
                </span>
                {stats != null && (
                  <span
                    className={
                      memOverLimit ? 'text-[#ef4444]' : 'text-[#808080]'
                    }
                  >
                    {stats.mem_limit_mb > 0
                      ? `${stats.mem_used_mb}MB / ${stats.mem_limit_mb}MB`
                      : `${stats.mem_used_mb}MB`}{' '}
                    | {stats.cpu_percent}% CPU
                  </span>
                )}
                {connectionStatus === 'disconnected' ? (
                  <button
                    type="button"
                    onClick={() => sessionRef.current?.reconnect()}
                    className="pointer-events-auto cursor-pointer border-0 bg-transparent p-0 text-[#808080] hover:text-[#cccccc]"
                    title="Reconnect terminal"
                    aria-label="Reconnect terminal"
                  >
                    ↻
                  </button>
                ) : null}
                <button
                  type="button"
                  onClick={() => sessionRef.current?.clear()}
                  className="pointer-events-auto cursor-pointer border-0 bg-transparent p-0 text-[#808080] leading-none hover:text-[#a0a0a0]"
                  title="Clear terminal"
                  aria-label="Clear terminal"
                >
                  🗑️
                </button>
              </div>
            </div>
          ) : null}
        </div>
      </div>
    </div>
  )
}
