/** Browser wiring for TerminalSession: real xterm, WebSocket and ResizeObserver. */
import { Terminal } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import '@xterm/xterm/css/xterm.css'
import { getBackendWsUrl } from './platform.js'
import { setDefaultTerminalDeps, TERMINAL_OPTIONS } from './terminalSession.js'

setDefaultTerminalDeps({
  createTerminal() {
    const term = new Terminal(TERMINAL_OPTIONS)
    const fitAddon = new FitAddon()
    term.loadAddon(fitAddon)
    return { term, fit: () => fitAddon.fit() }
  },
  WebSocket: globalThis.WebSocket,
  ResizeObserver: globalThis.ResizeObserver ?? null,
  wsUrl: getBackendWsUrl,
})
