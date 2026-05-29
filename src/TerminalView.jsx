import { useEffect, useRef } from 'react'
import { Terminal } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import '@xterm/xterm/css/xterm.css'

const ANSI_GREEN = '\x1b[32m'
const ANSI_RED = '\x1b[31m'
const ANSI_RESET = '\x1b[0m'

export default function TerminalView({ instanceId, isActive }) {
  const containerRef = useRef(null)

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

    const safeFit = () => {
      if (!alive) return
      try {
        fitAddon.fit()
      } catch {
        // Dimensions may be zero during layout
      }
    }
    safeFit()

    const wsUrl = `ws://127.0.0.1:8765/ws/pty/${encodeURIComponent(instanceId)}`
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
      term.dispose()
      if (
        ws.readyState === WebSocket.OPEN ||
        ws.readyState === WebSocket.CONNECTING
      ) {
        ws.close()
      }
    }
  }, [instanceId])

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
        <div ref={containerRef} className="min-h-0 min-w-0 flex-1" />
      </div>
    </div>
  )
}
