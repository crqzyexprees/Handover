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
  const timers = []
  return {
    terms,
    timers,
    /** Runs every pending timer callback (fake clock). */
    flushTimers() {
      while (timers.length) timers.shift().fn()
    },
    deps: {
      setTimeout: (fn, ms) => {
        const t = { fn, ms }
        timers.push(t)
        return t
      },
      clearTimeout: (t) => {
        const i = timers.indexOf(t)
        if (i >= 0) timers.splice(i, 1)
      },
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

const control = (obj) => `__handover_control__:${JSON.stringify(obj)}`
const lastWs = () => FakeWebSocket.instances.at(-1)

describe('TerminalSession resume protocol', () => {
  it('control frames are neither shown nor counted', () => {
    const { deps, terms } = makeDeps()
    new TerminalSession({ instanceId: 'a', container, deps })
    const ws = lastWs()
    ws.open()
    ws.receive(control({ type: 'attached', session: 7, from: 0 }))
    ws.receive('hello')
    expect(terms[0].written).toEqual(['hello'])
  })

  it('reconnect asks only for output it has not seen', () => {
    const { deps } = makeDeps()
    const session = new TerminalSession({ instanceId: 'a', container, deps })
    const ws = lastWs()
    expect(ws.url).toBe('ws://test/ws/pty/a')
    ws.open()
    ws.receive(control({ type: 'attached', session: 7, from: 0 }))
    ws.receive('hello ')
    ws.receive('→😀') // 3 string-length units
    session.reconnect()
    expect(lastWs().url).toBe('ws://test/ws/pty/a?session=7&since=9')
  })

  it('notes a new shell and a gap in output', () => {
    const { deps, terms } = makeDeps()
    const session = new TerminalSession({ instanceId: 'a', container, deps })
    lastWs().open()
    lastWs().receive(control({ type: 'attached', session: 1, from: 0 }))
    lastWs().receive('abc')
    session.reconnect()
    lastWs().open()
    lastWs().receive(control({ type: 'attached', session: 1, from: 10 }))
    session.reconnect()
    lastWs().open()
    lastWs().receive(control({ type: 'attached', session: 2, from: 0 }))
    const shown = terms[0].written.join('')
    expect(shown).toContain('some output was lost')
    expect(shown).toContain('new shell started')
  })

  it('auto-reconnects with backoff after an unexpected drop', () => {
    const { deps, timers, flushTimers } = makeDeps()
    new TerminalSession({ instanceId: 'a', container, deps })
    lastWs().open()
    lastWs().receive(control({ type: 'attached', session: 3, from: 0 }))
    lastWs().receive('xy')
    lastWs().onclose()
    expect(timers.map((t) => t.ms)).toEqual([500])
    flushTimers()
    expect(FakeWebSocket.instances).toHaveLength(2)
    expect(lastWs().url).toBe('ws://test/ws/pty/a?session=3&since=2')
    lastWs().onclose() // fails again before attaching
    expect(timers.map((t) => t.ms)).toEqual([1000])
  })

  it('gives up after five failed attempts', () => {
    const { deps, timers, flushTimers } = makeDeps()
    new TerminalSession({ instanceId: 'a', container, deps })
    for (let i = 0; i < 10; i++) {
      lastWs().onclose()
      flushTimers()
    }
    expect(FakeWebSocket.instances).toHaveLength(6) // first + 5 retries
    expect(timers).toHaveLength(0)
  })

  for (const type of ['exit', 'replaced', 'error']) {
    it(`does not auto-reconnect after "${type}"`, () => {
      const { deps, timers } = makeDeps()
      new TerminalSession({ instanceId: 'a', container, deps })
      lastWs().open()
      lastWs().receive(control({ type, message: 'boom' }))
      lastWs().onclose()
      expect(timers).toHaveLength(0)
    })
  }

  it('manual reconnect works after the shell exited', () => {
    const { deps } = makeDeps()
    const session = new TerminalSession({ instanceId: 'a', container, deps })
    lastWs().open()
    lastWs().receive(control({ type: 'exit' }))
    lastWs().onclose()
    session.reconnect()
    expect(FakeWebSocket.instances).toHaveLength(2)
  })

  it('dispose cancels a pending retry', () => {
    const { deps, timers } = makeDeps()
    const session = new TerminalSession({ instanceId: 'a', container, deps })
    lastWs().onclose()
    expect(timers).toHaveLength(1)
    session.dispose()
    expect(timers).toHaveLength(0)
  })
})
