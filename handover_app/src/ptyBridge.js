/**
 * Single WebSocket + onData attachment for the whole app.
 * React/HMR must never register term.onData directly — only this module does.
 */
import { getBackendWsUrl } from './platform.js'

const CONTROL_PREFIX = '__handover_control__:'
const BRIDGE_KEY = '__handover_pty_bridge__'

function getBridgeState() {
  if (!globalThis[BRIDGE_KEY]) {
    globalThis[BRIDGE_KEY] = {
      dataSub: null,
      resizeSub: null,
      ws: null,
      term: null,
      socketGeneration: 0,
      inputGeneration: 0,
    }
  }
  return globalThis[BRIDGE_KEY]
}

/** Drop ghost-handler bursts like "ba" / "bab"; allow keys, escapes, and bracketed paste. */
export function shouldForwardInput(data) {
  if (typeof data !== 'string' || data.length === 0) return false
  if (data.length === 1) return true
  if (data.startsWith('\x1b')) return true
  if (data.includes('\r') || data.includes('\n') || data.includes('\t')) return true
  return false
}

function closeSocket(state) {
  if (!state.ws) return
  state.ws.onopen = null
  state.ws.onmessage = null
  state.ws.onerror = null
  state.ws.onclose = null
  if (state.ws.readyState === WebSocket.OPEN || state.ws.readyState === WebSocket.CONNECTING) {
    state.ws.close()
  }
  state.ws = null
}

function detachInput(state) {
  state.dataSub?.dispose()
  state.resizeSub?.dispose()
  state.dataSub = null
  state.resizeSub = null
  state.inputGeneration += 1
}

export function disconnectPtyBridge() {
  const state = getBridgeState()
  state.socketGeneration += 1
  detachInput(state)
  closeSocket(state)
  state.term = null
}

/** Paste into the active PTY using bracketed paste (safe for shells). */
export function pasteIntoTerminal(text) {
  const state = getBridgeState()
  if (typeof text !== 'string' || text.length === 0) return
  if (!state.ws || state.ws.readyState !== WebSocket.OPEN) return
  const sanitized = text.replace(/\x1b/g, '')
  state.ws.send(`\x1b[200~${sanitized}\x1b[201~`)
}

export function connectPtyBridge({ instanceId, term, fitAddon, report }) {
  const state = getBridgeState()
  disconnectPtyBridge()

  state.term = term
  state.socketGeneration += 1
  const socketGeneration = state.socketGeneration
  state.inputGeneration += 1
  const inputGeneration = state.inputGeneration

  const sendResize = () => {
    if (
      state.inputGeneration !== inputGeneration ||
      state.socketGeneration !== socketGeneration ||
      !state.ws ||
      state.ws.readyState !== WebSocket.OPEN
    ) {
      return
    }
    state.ws.send(
      `${CONTROL_PREFIX}${JSON.stringify({
        type: 'resize',
        cols: term.cols,
        rows: term.rows,
      })}`,
    )
  }

  const connect = () => {
    if (state.inputGeneration !== inputGeneration || state.socketGeneration !== socketGeneration) {
      return
    }

    closeSocket(state)
    report('connecting')

    const ws = new WebSocket(getBackendWsUrl(`/ws/pty/${encodeURIComponent(instanceId)}`))
    ws.binaryType = 'arraybuffer'
    state.ws = ws

    ws.onopen = () => {
      if (state.inputGeneration !== inputGeneration || state.socketGeneration !== socketGeneration) {
        return
      }
      report('connected')
      try {
        fitAddon.fit()
        sendResize()
      } catch {
        // ignore
      }
      term.focus()
    }

    ws.onmessage = (event) => {
      if (state.inputGeneration !== inputGeneration || state.socketGeneration !== socketGeneration) {
        return
      }
      if (typeof event.data === 'string') {
        term.write(event.data)
      } else if (event.data instanceof ArrayBuffer) {
        term.write(new TextDecoder().decode(event.data))
      }
    }

    ws.onerror = () => {
      if (state.inputGeneration !== inputGeneration || state.socketGeneration !== socketGeneration) {
        return
      }
      report('disconnected')
    }

    ws.onclose = () => {
      if (state.inputGeneration !== inputGeneration || state.socketGeneration !== socketGeneration) {
        return
      }
      report('disconnected')
    }
  }

  state.dataSub = term.onData((data) => {
    if (state.inputGeneration !== inputGeneration) return
    if (!shouldForwardInput(data)) return
    if (!state.ws || state.ws.readyState !== WebSocket.OPEN) return
    state.ws.send(data)
  })

  state.resizeSub = term.onResize(() => {
    sendResize()
  })

  connect()

  return () => {
    if (state.inputGeneration !== inputGeneration) return
    disconnectPtyBridge()
    report('disconnected')
  }
}

if (import.meta.hot) {
  import.meta.hot.dispose(() => {
    disconnectPtyBridge()
  })
}
