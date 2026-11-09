/**
 * Linux-style terminal keyboard shortcuts for xterm.
 * Ctrl+Shift+C/V, Ctrl+Insert, Shift+Insert, select-all, font zoom.
 */
import { readClipboard, writeClipboard } from './clipboard.js'

const DEFAULT_FONT_SIZE = 14
const MIN_FONT_SIZE = 8
const MAX_FONT_SIZE = 32

async function copySelection(term) {
  if (!term.hasSelection()) return false
  const selection = term.getSelection()
  if (!selection) return false
  return writeClipboard(selection)
}

async function pasteFromClipboard(session) {
  pasteText(session, await readClipboard())
}

function pasteText(session, text) {
  if (!text) return
  session.paste(text)
  session.term.focus()
}

function isModifierOnly(event) {
  return event.key === 'Control' || event.key === 'Shift' || event.key === 'Alt' || event.key === 'Meta'
}

function handleZoom(term, delta) {
  const current = term.options.fontSize ?? DEFAULT_FONT_SIZE
  const next = Math.min(MAX_FONT_SIZE, Math.max(MIN_FONT_SIZE, current + delta))
  if (next !== current) {
    term.options.fontSize = next
  }
}

/**
 * @param {import('./terminalSession.js').TerminalSession} session
 * @param {HTMLElement} container
 * @returns {() => void}
 */
export function attachTerminalShortcuts(session, container) {
  const term = session.term
  term.attachCustomKeyEventHandler((event) => {
    if (event.type !== 'keydown' || isModifierOnly(event)) {
      return true
    }

    const ctrl = event.ctrlKey || event.metaKey
    const shift = event.shiftKey
    const key = event.key

    // Ctrl+Shift+C — copy (Linux terminal default)
    if (ctrl && shift && (key === 'c' || key === 'C')) {
      event.preventDefault()
      void copySelection(term)
      return false
    }

    // Ctrl+C with selection — copy instead of sending SIGINT
    if (ctrl && !shift && (key === 'c' || key === 'C') && term.hasSelection()) {
      event.preventDefault()
      void copySelection(term)
      return false
    }

    // Ctrl+Shift+V — paste
    if (ctrl && shift && (key === 'v' || key === 'V')) {
      event.preventDefault()
      void pasteFromClipboard(session)
      return false
    }

    // Ctrl+Shift+A — select all
    if (ctrl && shift && (key === 'a' || key === 'A')) {
      event.preventDefault()
      term.selectAll()
      return false
    }

    // Ctrl+Insert — copy
    if (ctrl && !shift && key === 'Insert') {
      event.preventDefault()
      void copySelection(term)
      return false
    }

    // Shift+Insert — paste
    if (!ctrl && shift && key === 'Insert') {
      event.preventDefault()
      void pasteFromClipboard(session)
      return false
    }

    // Ctrl+Shift+Plus / Ctrl+Shift+= — increase font
    if (ctrl && shift && (key === '+' || key === '=')) {
      event.preventDefault()
      handleZoom(term, 1)
      return false
    }

    // Ctrl+Shift+Minus — decrease font
    if (ctrl && shift && key === '-') {
      event.preventDefault()
      handleZoom(term, -1)
      return false
    }

    // Ctrl+Shift+0 — reset font
    if (ctrl && shift && key === '0') {
      event.preventDefault()
      term.options.fontSize = DEFAULT_FONT_SIZE
      return false
    }

    // Ctrl+Shift+Home — scroll to top
    if (ctrl && shift && key === 'Home') {
      event.preventDefault()
      term.scrollToTop()
      return false
    }

    // Ctrl+Shift+End — scroll to bottom
    if (ctrl && shift && key === 'End') {
      event.preventDefault()
      term.scrollToBottom()
      return false
    }

    return true
  })

  const onMouseDown = (event) => {
    // Middle-click paste (Linux). The webview can't read the X11/Wayland
    // primary selection, so approximate it: paste this terminal's current
    // selection if there is one, otherwise the clipboard.
    if (event.button === 1) {
      event.preventDefault()
      const selection = term.hasSelection() ? term.getSelection() : ''
      if (selection) {
        pasteText(session, selection)
      } else {
        void pasteFromClipboard(session)
      }
    }
    // Right-click copy is handled once, in onContextMenu.
  }

  const onContextMenu = (event) => {
    if (term.hasSelection()) {
      event.preventDefault()
      void copySelection(term)
    }
  }

  container.addEventListener('mousedown', onMouseDown)
  container.addEventListener('contextmenu', onContextMenu)

  return () => {
    term.attachCustomKeyEventHandler(() => true)
    container.removeEventListener('mousedown', onMouseDown)
    container.removeEventListener('contextmenu', onContextMenu)
  }
}
