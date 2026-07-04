import { useEffect, useRef, useState } from 'react'
import { Terminal } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import '@xterm/xterm/css/xterm.css'
import { getInstanceStats } from './api'
import { connectPtyBridge } from './ptyBridge.js'

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

export default function TerminalView({ instanceId, onConnectionChange }) {
  const containerRef = useRef(null)
  const termRef = useRef(null)
  const onConnectionChangeRef = useRef(onConnectionChange)
  const [statsState, setStatsState] = useState(null)
  const [connectionStatus, setConnectionStatus] = useState('disconnected')
  const [reconnectNonce, setReconnectNonce] = useState(0)

  useEffect(() => {
    onConnectionChangeRef.current = onConnectionChange
  }, [onConnectionChange])

  useEffect(() => {
    if (instanceId == null || instanceId === '') return

    const container = containerRef.current
    if (!container) return

    container.replaceChildren()

    const term = new Terminal({
      cursorBlink: true,
      fontSize: 14,
      fontFamily: 'JetBrains Mono, Fira Code, monospace',
      theme: {
        background: '#0f1117',
        foreground: '#e2e8f0',
        cursor: '#60a5fa',
      },
    })

    const fitAddon = new FitAddon()
    term.loadAddon(fitAddon)
    term.open(container)
    termRef.current = term

    const resizeObserver = new ResizeObserver(() => {
      try {
        fitAddon.fit()
      } catch {
        // ignore zero-size layout
      }
    })
    resizeObserver.observe(container)

    const report = (status) => {
      setConnectionStatus(status)
      onConnectionChangeRef.current?.(instanceId, status)
    }

    const disconnectBridge = connectPtyBridge({
      instanceId,
      term,
      fitAddon,
      report,
    })

    const focusTimer = window.setTimeout(() => {
      try {
        term.focus()
        fitAddon.fit()
      } catch {
        // ignore
      }
    }, 50)

    return () => {
      window.clearTimeout(focusTimer)
      resizeObserver.disconnect()
      disconnectBridge()
      termRef.current = null
      term.dispose()
    }
  }, [instanceId, reconnectNonce])

  useEffect(() => {
    if (instanceId == null || instanceId === '') return

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
  }, [instanceId])

  const stats = statsState?.instanceId === instanceId ? statsState.data : null

  const memOverLimit =
    stats != null &&
    stats.mem_limit_mb > 0 &&
    stats.mem_used_mb / stats.mem_limit_mb > 0.8

  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col">
      <div className="box-border flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden bg-[#1e1e1e]">
        <div
          className="relative min-h-0 min-w-0 flex-1"
          onClick={() => termRef.current?.focus()}
        >
          <div ref={containerRef} className="h-full min-h-0 min-w-0" />
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
                  {stats.mem_used_mb}MB / {stats.mem_limit_mb}MB |{' '}
                  {stats.cpu_percent}% CPU
                </span>
              )}
              {connectionStatus === 'disconnected' ? (
                <button
                  type="button"
                  onClick={() => setReconnectNonce((n) => n + 1)}
                  className="pointer-events-auto cursor-pointer border-0 bg-transparent p-0 text-[#808080] hover:text-[#cccccc]"
                  title="Reconnect terminal"
                  aria-label="Reconnect terminal"
                >
                  ↻
                </button>
              ) : null}
              <button
                type="button"
                onClick={() => termRef.current?.clear()}
                className="pointer-events-auto cursor-pointer border-0 bg-transparent p-0 text-[#808080] leading-none hover:text-[#a0a0a0]"
                title="Clear terminal"
                aria-label="Clear terminal"
              >
                🗑️
              </button>
            </div>
          </div>
        </div>
      </div>
    </div>
  )
}
