/**
 * TerminalSession: one xterm + one PTY WebSocket per terminal instance.
 *
 * This module is the only place that wires keystrokes, pastes and output
 * between xterm and the backend PTY (native or Docker; same /ws/pty/{id}).
 * Every byte the user sends goes through `term.onData` → `#send`, so there is
 * exactly one write path. A registry keyed by instance id guarantees at most
 * one live session per instance across React remounts and HMR.
 */
const CONTROL_PREFIX = '__handover_control__:'
const REGISTRY_KEY = '__handover_terminal_sessions__'

export const TERMINAL_OPTIONS = {
  cursorBlink: true,
  fontSize: 14,
  fontFamily: 'JetBrains Mono, Fira Code, monospace',
  rightClickSelectsWord: true,
  scrollback: 5000,
  theme: {
    background: '#0f1117',
    foreground: '#e2e8f0',
    cursor: '#60a5fa',
    selectionBackground: '#264f78',
  },
}

function registry() {
  if (!globalThis[REGISTRY_KEY]) {
    globalThis[REGISTRY_KEY] = new Map()
  }
  return globalThis[REGISTRY_KEY]
}

/** The live session for an instance, if any. */
export function getTerminalSession(instanceId) {
  return registry().get(String(instanceId ?? '')) ?? null
}

/** Dispose every live session (HMR / app teardown). */
export function disposeAllTerminalSessions() {
  for (const session of [...registry().values()]) {
    session.dispose()
  }
}

/**
 * @typedef {'connecting' | 'connected' | 'disconnected'} ConnectionStatus
 *
 * @typedef {object} SessionDeps  Injected so tests can run without a browser.
 * @property {() => {term: any, fit: () => void}} createTerminal
 * @property {typeof WebSocket} WebSocket
 * @property {typeof ResizeObserver | null} ResizeObserver
 * @property {(path: string) => string} wsUrl
 */

/** @type {SessionDeps} */
let defaultDeps = null

/** Set by terminalSessionDeps.js (browser build); kept separate so tests don't load xterm. */
export function setDefaultTerminalDeps(deps) {
  defaultDeps = deps
}

export class TerminalSession {
  #instanceId
  #deps
  #term
  #fit
  #ws = null
  #disposables = []
  #status = 'disconnected'
  #statusListeners = new Set()
  #disposed = false
  #decoder = new TextDecoder()

  /**
   * @param {object} opts
   * @param {string} opts.instanceId
   * @param {HTMLElement} opts.container
   * @param {SessionDeps} [opts.deps]
   */
  constructor({ instanceId, container, deps }) {
    this.#instanceId = String(instanceId)
    this.#deps = deps ?? defaultDeps
    if (!this.#deps) {
      throw new Error('TerminalSession deps not configured')
    }

    // At most one session per instance: replace any survivor from a remount.
    registry().get(this.#instanceId)?.dispose()
    registry().set(this.#instanceId, this)

    const { term, fit } = this.#deps.createTerminal()
    this.#term = term
    this.#fit = fit
    container.replaceChildren?.()
    term.open(container)

    this.#disposables.push(
      term.onData((data) => this.#send(data)),
      term.onResize(() => this.#sendResize()),
    )

    if (this.#deps.ResizeObserver) {
      const observer = new this.#deps.ResizeObserver(() => this.fit())
      observer.observe(container)
      this.#disposables.push({ dispose: () => observer.disconnect() })
    }

    this.#connect()
  }

  get instanceId() {
    return this.#instanceId
  }

  /** The underlying xterm instance (for shortcuts / selection). */
  get term() {
    return this.#term
  }

  /** @returns {ConnectionStatus} */
  get status() {
    return this.#status
  }

  /** Subscribe to status changes; returns an unsubscribe function. */
  onStatus(listener) {
    this.#statusListeners.add(listener)
    return () => this.#statusListeners.delete(listener)
  }

  /**
   * Paste text as the user would. xterm normalizes newlines and adds
   * bracketed-paste markers only when the running program enabled that
   * mode, then emits it through onData → the single write path.
   */
  paste(text) {
    if (this.#disposed || typeof text !== 'string' || text.length === 0) return
    this.#term.paste(text)
  }

  /** Reopen the PTY socket, keeping xterm and its scrollback. */
  reconnect() {
    if (this.#disposed) return
    this.#term.write('\r\n\x1b[90m[handover] reconnecting…\x1b[0m\r\n')
    this.#connect()
  }

  fit() {
    if (this.#disposed) return
    try {
      this.#fit()
    } catch {
      // zero-size layout while hidden or settling
    }
  }

  focus() {
    if (this.#disposed) return
    this.fit()
    this.#term.refresh?.(0, Math.max(0, this.#term.rows - 1))
    this.#term.focus()
  }

  clear() {
    if (!this.#disposed) this.#term.clear()
  }

  dispose() {
    if (this.#disposed) return
    this.#disposed = true
    this.#closeSocket()
    for (const d of this.#disposables) d.dispose()
    this.#disposables = []
    this.#setStatus('disconnected')
    this.#statusListeners.clear()
    this.#term.dispose()
    if (registry().get(this.#instanceId) === this) {
      registry().delete(this.#instanceId)
    }
  }

  // ── internals ────────────────────────────────────────────────────────────

  #setStatus(status) {
    if (this.#status === status) return
    this.#status = status
    for (const listener of this.#statusListeners) listener(status)
  }

  #send(data) {
    if (typeof data !== 'string' || data.length === 0) return
    const ws = this.#ws
    if (!ws || ws.readyState !== this.#deps.WebSocket.OPEN) return
    ws.send(data)
  }

  #sendResize() {
    const { cols, rows } = this.#term
    if (!cols || !rows) return
    this.#send(`${CONTROL_PREFIX}${JSON.stringify({ type: 'resize', cols, rows })}`)
  }

  #closeSocket() {
    const ws = this.#ws
    if (!ws) return
    this.#ws = null
    ws.onopen = ws.onmessage = ws.onerror = ws.onclose = null
    const { OPEN, CONNECTING } = this.#deps.WebSocket
    if (ws.readyState === OPEN || ws.readyState === CONNECTING) ws.close()
  }

  #connect() {
    this.#closeSocket()
    this.#decoder = new TextDecoder()
    this.#setStatus('connecting')

    const url = this.#deps.wsUrl(`/ws/pty/${encodeURIComponent(this.#instanceId)}`)
    const ws = new this.#deps.WebSocket(url)
    ws.binaryType = 'arraybuffer'
    this.#ws = ws
    // Handlers check identity so a superseded socket can never write.
    const current = () => this.#ws === ws && !this.#disposed

    ws.onopen = () => {
      if (!current()) return
      this.#setStatus('connected')
      this.fit()
      this.#sendResize()
    }
    ws.onmessage = (event) => {
      if (!current()) return
      if (typeof event.data === 'string') {
        this.#term.write(event.data)
      } else if (event.data instanceof ArrayBuffer) {
        // stream: true keeps a split multi-byte character for the next chunk.
        this.#term.write(this.#decoder.decode(event.data, { stream: true }))
      }
    }
    ws.onerror = () => {
      if (current()) this.#setStatus('disconnected')
    }
    ws.onclose = () => {
      if (current()) this.#setStatus('disconnected')
    }
  }
}

if (import.meta.hot) {
  import.meta.hot.dispose(() => disposeAllTerminalSessions())
}
