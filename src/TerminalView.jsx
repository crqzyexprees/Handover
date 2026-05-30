import { useEffect, useRef, useState } from 'react'
import { Terminal } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import { WebglAddon } from '@xterm/addon-webgl'
import '@xterm/xterm/css/xterm.css'
import { getInstanceStats } from './api'

const ANSI_GREEN = '\x1b[32m'
const ANSI_RED = '\x1b[31m'
const ANSI_RESET = '\x1b[0m'

const STATS_POLL_MS = 5000

export default function TerminalView({ instanceId, isActive }) {
  const containerRef = useRef(null)
  const termRef = useRef(null)
  const [stats, setStats] = useState(null)

  useEffect(() => {
    if (instanceId == null || instanceId === '') {
      return
    }

    const container = containerRef.current
    if (!container) {
      return
    }

    let alive = true

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

    const safeFit = () => {
      if (!alive) return
      try {
        fitAddon.fit()
      } catch {
        // Dimensions may be zero during layout
      }
    }
    safeFit()

    const urlParams = new URLSearchParams(window.location.search)
    const port = urlParams.get('port') || '8765'
    const wsUrl = `ws://127.0.0.1:${port}/ws/pty/${encodeURIComponent(instanceId)}`
    const ws = new WebSocket(wsUrl)
    ws.binaryType = 'arraybuffer'

    const writeFromSocket = (data) => {
      if (typeof data === 'string') {
        term.write(data)
      } else if (data instanceof ArrayBuffer) {
        term.write(new TextDecoder().decode(data))
      }
    }

    ws.onopen = () => {
      if (!alive) return
      term.writeln(`${ANSI_GREEN}● Connected${ANSI_RESET}`)
    }

    ws.onmessage = (event) => {
      if (!alive) return
      writeFromSocket(event.data)
    }

    ws.onerror = () => {
      if (!alive) return
      term.writeln(`${ANSI_RED}● Connection error${ANSI_RESET}`)
    }

    ws.onclose = () => {
      if (!alive) return
      term.writeln(`${ANSI_RED}● Disconnected${ANSI_RESET}`)
    }

    const dataSub = term.onData((data) => {
      if (!alive) return
      if (ws.readyState === WebSocket.OPEN) {
        ws.send(data)
      }
    })

    const resizeObserver = new ResizeObserver(() => {
      safeFit()
    })
    resizeObserver.observe(container)

    return () => {
      alive = false
      resizeObserver.disconnect()
      dataSub.dispose()
      if (webglAddon) {
        try {
          webglAddon.dispose()
        } catch {
          // Already disposed after context loss
        }
      }
      termRef.current = null
      term.dispose()
      if (
        ws.readyState === WebSocket.OPEN ||
        ws.readyState === WebSocket.CONNECTING
      ) {
        ws.close()
      }
    }
  }, [instanceId])

  useEffect(() => {
    if (instanceId == null || instanceId === '') {
      setStats(null)
      return
    }

    let cancelled = false
    setStats(null)

    const fetchStats = async () => {
      const { data, error } = await getInstanceStats(instanceId)
      if (!cancelled && data && !error) {
        setStats(data)
      }
    }

    fetchStats()
    const intervalId = setInterval(fetchStats, STATS_POLL_MS)

    return () => {
      cancelled = true
      clearInterval(intervalId)
    }
  }, [instanceId])

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
