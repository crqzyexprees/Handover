import { afterEach, describe, expect, it } from 'vitest'
import {
  TerminalSession,
  disposeAllTerminalSessions,
  getTerminalSession,
} from './terminalSession.js'

class FakeTerminal {
  cols = 80
  rows = 24
  written = []
  disposed = false
  #dataListeners = new Set()
  #resizeListeners = new Set()

  open() {}
  focus() {}
  refresh() {}
  clear() {}
  write(text) {
    this.written.push(text)
  }
  dispose() {
    this.disposed = true
  }
  onData(cb) {
    this.#dataListeners.add(cb)
    return { dispose: () => this.#dataListeners.delete(cb) }
  }
  onResize(cb) {
    this.#resizeListeners.add(cb)
    return { dispose: () => this.#resizeListeners.delete(cb) }
  }
  /** Simulates the user typing (xterm emits onData). */
  type(data) {
    for (const cb of this.#dataListeners) cb(data)
  }
  /** Real xterm.paste emits through onData too. */
  paste(text) {
    this.type(text.replace(/\r?\n/g, '\r'))
  }
}

class FakeWebSocket {
  static CONNECTING = 0
  static OPEN = 1
  static CLOSED = 3
  static instances = []

  readyState = FakeWebSocket.CONNECTING
  sent = []

  constructor(url) {
    this.url = url
    FakeWebSocket.instances.push(this)
  }
  send(data) {
    this.sent.push(data)
  }
  close() {
    this.readyState = FakeWebSocket.CLOSED
  }
  open() {
    this.readyState = FakeWebSocket.OPEN
    this.onopen?.()
  }
  receive(data) {
    this.onmessage?.({ data })
  }
}

function makeDeps() {
  const terms = []
  return {
    terms,
    deps: {
      createTerminal() {
        const term = new FakeTerminal()
        terms.push(term)
        return { term, fit: () => {} }
      },
      WebSocket: FakeWebSocket,
      ResizeObserver: null,
      wsUrl: (path) => `ws://test${path}`,
    },
  }
}

const container = { replaceChildren() {} }
const userSends = (ws) => ws.sent.filter((d) => !d.startsWith('__handover_control__:'))

afterEach(() => {
  disposeAllTerminalSessions()
  FakeWebSocket.instances = []
})

describe('TerminalSession', () => {
  it('sends one keystroke exactly once', () => {
    const { deps, terms } = makeDeps()
    new TerminalSession({ instanceId: 'a', container, deps })
    const ws = FakeWebSocket.instances[0]
    ws.open()
    terms[0].type('x')
    expect(userSends(ws)).toEqual(['x'])
  })

  it('drops input until the socket is open', () => {
    const { deps, terms } = makeDeps()
    new TerminalSession({ instanceId: 'a', container, deps })
    terms[0].type('early')
    const ws = FakeWebSocket.instances[0]
    ws.open()
    expect(userSends(ws)).toEqual([])
  })

  it('replaces a previous session for the same instance (remount)', () => {
    const { deps, terms } = makeDeps()
    new TerminalSession({ instanceId: 'a', container, deps })
    const second = new TerminalSession({ instanceId: 'a', container, deps })
    const [ws1, ws2] = FakeWebSocket.instances
    expect(ws1.readyState).toBe(FakeWebSocket.CLOSED)
    expect(terms[0].disposed).toBe(true)
    ws2.open()
    terms[0].type('stale') // old xterm must not reach any socket
    terms[1].type('y')
    expect(userSends(ws1)).toEqual([])
    expect(userSends(ws2)).toEqual(['y'])
    expect(getTerminalSession('a')).toBe(second)
  })

  it('reconnect keeps the terminal and ignores the superseded socket', () => {
    const { deps, terms } = makeDeps()
    const session = new TerminalSession({ instanceId: 'a', container, deps })
    const ws1 = FakeWebSocket.instances[0]
    ws1.open()
    session.reconnect()
    const ws2 = FakeWebSocket.instances[1]
    expect(ws1.readyState).toBe(FakeWebSocket.CLOSED)
    expect(terms).toHaveLength(1)
    expect(terms[0].disposed).toBe(false)
    ws1.receive('old output')
    ws2.open()
    terms[0].type('z')
    expect(terms[0].written).not.toContain('old output')
    expect(userSends(ws2)).toEqual(['z'])
  })

  it('paste goes through the single write path once', () => {
    const { deps } = makeDeps()
    const session = new TerminalSession({ instanceId: 'a', container, deps })
    const ws = FakeWebSocket.instances[0]
    ws.open()
    session.paste('line1\nline2')
    expect(userSends(ws)).toEqual(['line1\rline2'])
  })

  it('reports status transitions', () => {
    const { deps } = makeDeps()
    const session = new TerminalSession({ instanceId: 'a', container, deps })
    const seen = []
    session.onStatus((s) => seen.push(s))
    const ws = FakeWebSocket.instances[0]
    ws.open()
    ws.onclose()
    expect(seen).toEqual(['connected', 'disconnected'])
  })

  it('decodes a UTF-8 character split across binary frames', () => {
    const { deps, terms } = makeDeps()
    new TerminalSession({ instanceId: 'a', container, deps })
    const ws = FakeWebSocket.instances[0]
    ws.open()
    const bytes = new TextEncoder().encode('a→b')
    ws.receive(bytes.slice(0, 2).buffer)
    ws.receive(bytes.slice(2).buffer)
    expect(terms[0].written.join('')).toBe('a→b')
  })

  it('dispose closes the socket and unregisters', () => {
    const { deps, terms } = makeDeps()
    const session = new TerminalSession({ instanceId: 'a', container, deps })
    const ws = FakeWebSocket.instances[0]
    ws.open()
    session.dispose()
    terms[0].type('after')
    expect(ws.readyState).toBe(FakeWebSocket.CLOSED)
    expect(userSends(ws)).toEqual([])
    expect(getTerminalSession('a')).toBeNull()
  })
})
