import { useCallback, useEffect, useRef, useState } from 'react'
import { Terminal } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import '@xterm/xterm/css/xterm.css'
import { getBackendWsUrl } from './platform.js'
import { getInstanceStats } from './api'

const STATS_POLL_MS = 5000
const CONTROL_PREFIX = '__handover_control__:'

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
  const fitAddonRef = useRef(null)
  const wsRef = useRef(null)
  const dataSubRef = useRef(null)
  const resizeSubRef = useRef(null)
  const isActiveRef = useRef(isActive)
  const onConnectionChangeRef = useRef(onConnectionChange)
  const [statsState, setStatsState] = useState(null)
  const [connectionStatus, setConnectionStatus] = useState('disconnected')
  const [reconnectNonce, setReconnectNonce] = useState(0)

  useEffect(() => {
    onConnectionChangeRef.current = onConnectionChange
  }, [onConnectionChange])

  const reportConnection = useCallback((status) => {
    setConnectionStatus(status)
    onConnectionChangeRef.current?.(instanceId, status)
  }, [instanceId])

  useEffect(() => {
    isActiveRef.current = isActive
  }, [isActive])

  // xterm lifecycle — one terminal per instanceId
  useEffect(() => {
    if (instanceId == null || instanceId === '') {
      return
    }

    const container = containerRef.current
    if (!container) {
      return
    }

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
    fitAddonRef.current = fitAddon

    const resizeObserver = new ResizeObserver(() => {
      if (!isActiveRef.current) return
      try {
        fitAddon.fit()
      } catch {
        // Hidden tabs can report zero size
      }
    })
    resizeObserver.observe(container)

    return () => {
      resizeObserver.disconnect()
      dataSubRef.current?.dispose()
      resizeSubRef.current?.dispose()
      dataSubRef.current = null
      resizeSubRef.current = null
      if (wsRef.current) {
        wsRef.current.onopen = null
        wsRef.current.onmessage = null
        wsRef.current.onerror = null
        wsRef.current.onclose = null
        if (
          wsRef.current.readyState === WebSocket.OPEN ||
          wsRef.current.readyState === WebSocket.CONNECTING
        ) {
          wsRef.current.close()
        }
        wsRef.current = null
      }
      fitAddonRef.current = null
      termRef.current = null
      term.dispose()
    }
  }, [instanceId])

  // WebSocket — only while this tab is active (prevents stacked handlers)
  useEffect(() => {
    if (instanceId == null || instanceId === '' || !isActive) {
      return
    }

    const term = termRef.current
    const fitAddon = fitAddonRef.current
    if (!term || !fitAddon) {
      return
    }

    let disposed = false
    let ws = null
    let socketGeneration = 0

    dataSubRef.current?.dispose()
    resizeSubRef.current?.dispose()
    dataSubRef.current = null
    resizeSubRef.current = null
    if (wsRef.current) {
      wsRef.current.onopen = null
      wsRef.current.onmessage = null
      wsRef.current.onerror = null
      wsRef.current.onclose = null
      if (
        wsRef.current.readyState === WebSocket.OPEN ||
        wsRef.current.readyState === WebSocket.CONNECTING
      ) {
        wsRef.current.close()
      }
      wsRef.current = null
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

    const closeSocket = () => {
      if (!ws) return
      const closingWs = ws
      ws.onopen = null
      ws.onmessage = null
      ws.onerror = null
      ws.onclose = null
      if (ws.readyState === WebSocket.OPEN || ws.readyState === WebSocket.CONNECTING) {
        ws.close()
      }
      ws = null
      if (wsRef.current === closingWs) {
        wsRef.current = null
      }
    }

    const connect = () => {
      if (disposed) return

      closeSocket()
      socketGeneration += 1
      const myGeneration = socketGeneration

      reportConnection('connecting')

      const wsUrl = getBackendWsUrl(`/ws/pty/${encodeURIComponent(instanceId)}`)
      const nextWs = new WebSocket(wsUrl)
      nextWs.binaryType = 'arraybuffer'
      ws = nextWs
      wsRef.current = nextWs

      nextWs.onopen = () => {
        if (disposed || myGeneration !== socketGeneration) return
        reportConnection('connected')
        safeFit()
        sendResize()
      }

      nextWs.onmessage = (event) => {
        if (disposed || myGeneration !== socketGeneration) return
        if (typeof event.data === 'string') {
          term.write(event.data)
        } else if (event.data instanceof ArrayBuffer) {
          term.write(new Uint8Array(event.data))
        }
      }

      nextWs.onerror = () => {
        if (disposed || myGeneration !== socketGeneration) return
        reportConnection('disconnected')
      }

      nextWs.onclose = () => {
        if (disposed || myGeneration !== socketGeneration) return
        reportConnection('disconnected')
      }
    }

    const dataSub = term.onData((data) => {
      if (disposed || !ws || ws.readyState !== WebSocket.OPEN) return
      ws.send(data)
    })
    dataSubRef.current = dataSub

    const resizeSub = term.onResize(() => {
      sendResize()
    })
    resizeSubRef.current = resizeSub

    connect()

    return () => {
      disposed = true
      socketGeneration += 1
      dataSub.dispose()
      resizeSub.dispose()
      if (dataSubRef.current === dataSub) {
        dataSubRef.current = null
      }
      if (resizeSubRef.current === resizeSub) {
        resizeSubRef.current = null
      }
      closeSocket()
      reportConnection('disconnected')
    }
  }, [instanceId, isActive, reconnectNonce, reportConnection])

  useEffect(() => {
    if (!isActive) return
    const timer = window.setTimeout(() => {
      try {
        termRef.current?.focus()
        fitAddonRef.current?.fit()
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
              {connectionStatus === 'disconnected' && isActive ? (
                <button
                  type="button"
                  onClick={() => setReconnectNonce((value) => value + 1)}
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
