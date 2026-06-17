import { useCallback, useEffect, useRef, useState } from 'react'
import { Terminal } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import { WebglAddon } from '@xterm/addon-webgl'
import '@xterm/xterm/css/xterm.css'
import { getBackendWsUrl } from './platform.js'
import { getInstanceStats } from './api'

const STATS_POLL_MS = 5000
const CONTROL_PREFIX = '__handover_control__:'
const RECONNECT_MS = 1500

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

export default function TerminalView({ instanceId, isActive, onConnectionChange }) {
  const containerRef = useRef(null)
  const termRef = useRef(null)
  const onConnectionChangeRef = useRef(onConnectionChange)
  const [statsState, setStatsState] = useState(null)
  const [connectionStatus, setConnectionStatus] = useState('connecting')

  useEffect(() => {
    onConnectionChangeRef.current = onConnectionChange
  }, [onConnectionChange])

  const reportConnection = useCallback((status) => {
    setConnectionStatus(status)
    onConnectionChangeRef.current?.(instanceId, status)
  }, [instanceId])

  useEffect(() => {
    if (instanceId == null || instanceId === '') {
      return
    }

    const container = containerRef.current
    if (!container) {
      return
    }

    let disposed = false
    let reconnectTimer = null
    let ws = null

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

    let webglAddon = null
    try {
      webglAddon = new WebglAddon()
      webglAddon.onContextLoss(() => {
        webglAddon?.dispose()
        webglAddon = null
      })
      term.loadAddon(webglAddon)
    } catch {
      webglAddon = null
    }

    const sendResize = () => {
      if (disposed || !ws || ws.readyState !== WebSocket.OPEN) return
      ws.send(
        `${CONTROL_PREFIX}${JSON.stringify({
          type: 'resize',
          cols: term.cols,
          rows: term.rows,
        })}`,
      )
    }

    const safeFit = () => {
      if (disposed) return
      try {
        fitAddon.fit()
        sendResize()
      } catch {
        // Dimensions may be zero while the tab is hidden
      }
    }

    const scheduleReconnect = () => {
      if (disposed) return
      reconnectTimer = window.setTimeout(() => {
        reconnectTimer = null
        connect()
      }, RECONNECT_MS)
    }

    const connect = () => {
      if (disposed) return

      if (ws) {
        ws.onopen = null
        ws.onmessage = null
        ws.onerror = null
        ws.onclose = null
        if (ws.readyState === WebSocket.OPEN || ws.readyState === WebSocket.CONNECTING) {
          ws.close()
        }
        ws = null
      }

      reportConnection('connecting')

      const wsUrl = getBackendWsUrl(`/ws/pty/${encodeURIComponent(instanceId)}`)
      ws = new WebSocket(wsUrl)
      ws.binaryType = 'arraybuffer'

      ws.onopen = () => {
        if (disposed) return
        reportConnection('connected')
        safeFit()
        sendResize()
      }

      ws.onmessage = (event) => {
        if (disposed) return
        if (typeof event.data === 'string') {
          term.write(event.data)
        } else if (event.data instanceof ArrayBuffer) {
          term.write(new TextDecoder().decode(event.data))
        }
      }

      ws.onerror = () => {
        if (disposed) return
        reportConnection('disconnected')
      }

      ws.onclose = () => {
        if (disposed) return
        reportConnection('disconnected')
        scheduleReconnect()
      }
    }

    const dataSub = term.onData((data) => {
      if (disposed || !ws || ws.readyState !== WebSocket.OPEN) return
      ws.send(data)
    })

    const resizeSub = term.onResize(() => {
      sendResize()
    })

    const resizeObserver = new ResizeObserver(() => {
      safeFit()
    })
    resizeObserver.observe(container)

    connect()

    return () => {
      disposed = true
      if (reconnectTimer != null) {
        window.clearTimeout(reconnectTimer)
      }
      resizeObserver.disconnect()
      dataSub.dispose()
      resizeSub.dispose()
      if (webglAddon) {
        try {
          webglAddon.dispose()
        } catch {
          // Already disposed after context loss
        }
      }
      if (ws) {
        ws.onopen = null
        ws.onmessage = null
        ws.onerror = null
        ws.onclose = null
        if (ws.readyState === WebSocket.OPEN || ws.readyState === WebSocket.CONNECTING) {
          ws.close()
        }
      }
      termRef.current = null
      term.dispose()
      reportConnection('disconnected')
    }
  }, [instanceId, reportConnection])

  useEffect(() => {
    if (!isActive) return
    const timer = window.setTimeout(() => {
      try {
        termRef.current?.focus()
      } catch {
        // Terminal may not be ready yet
      }
    }, 0)
    return () => window.clearTimeout(timer)
  }, [isActive, instanceId])

  useEffect(() => {
    if (instanceId == null || instanceId === '') {
      return
    }

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
    <div
      style={{
        display: isActive ? 'flex' : 'none',
        flexDirection: 'column',
        height: '100%',
        width: '100%',
      }}
      className="min-h-0 min-w-0 flex-1"
    >
      <div className="box-border flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden bg-[#1e1e1e]">
        <div className="relative min-h-0 min-w-0 flex-1">
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
