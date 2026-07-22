/**
 * Linux-style terminal keyboard shortcuts for xterm.
 * Ctrl+Shift+C/V, Ctrl+Insert, Shift+Insert, select-all, font zoom.
 */
import { pasteIntoTerminal } from './ptyBridge.js'

const DEFAULT_FONT_SIZE = 14
const MIN_FONT_SIZE = 8
const MAX_FONT_SIZE = 32

async function readClipboardText() {
  if (navigator.clipboard?.readText) {
    return navigator.clipboard.readText()
  }
  return ''
}

async function writeClipboardText(text) {
  if (navigator.clipboard?.writeText) {
    await navigator.clipboard.writeText(text)
    return
  }
  const textarea = document.createElement('textarea')
  textarea.value = text
  textarea.style.position = 'fixed'
  textarea.style.left = '-9999px'
  document.body.appendChild(textarea)
  textarea.select()
  document.execCommand('copy')
  document.body.removeChild(textarea)
}

async function copySelection(term) {
  const selection = term.getSelection()
  if (!selection) return
  await writeClipboardText(selection)
}

async function pasteFromClipboard(term) {
  const text = await readClipboardText()
  if (!text) return
  pasteIntoTerminal(text)
  term.focus()
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
 * @param {import('@xterm/xterm').Terminal} term
 * @param {HTMLElement} container
 * @returns {() => void}
 */
export function attachTerminalShortcuts(term, container) {
  term.attachCustomKeyEventHandler((event) => {
    if (event.type !== 'keydown' || isModifierOnly(event)) {
      return true
    }

    const ctrl = event.ctrlKey || event.metaKey
    const shift = event.shiftKey

    // Ctrl+Shift+C — copy (Linux terminal default)
    if (ctrl && shift && (event.key === 'c' || event.key === 'C')) {
      event.preventDefault()
      void copySelection(term)
      return false
    }

    // Ctrl+Shift+V — paste
    if (ctrl && shift && (event.key === 'v' || event.key === 'V')) {
      event.preventDefault()
      void pasteFromClipboard(term)
      return false
    }

    // Ctrl+Shift+A — select all
    if (ctrl && shift && (event.key === 'a' || event.key === 'A')) {
      event.preventDefault()
      term.selectAll()
      return false
    }

    // Ctrl+Insert — copy
    if (ctrl && !shift && event.key === 'Insert') {
      event.preventDefault()
      void copySelection(term)
      return false
    }

    // Shift+Insert — paste
    if (!ctrl && shift && event.key === 'Insert') {
      event.preventDefault()
      void pasteFromClipboard(term)
      return false
    }

    // Ctrl+Shift+Plus / Ctrl+Shift+= — increase font
    if (ctrl && shift && (event.key === '+' || event.key === '=')) {
      event.preventDefault()
      handleZoom(term, 1)
      return false
    }

    // Ctrl+Shift+Minus — decrease font
    if (ctrl && shift && event.key === '-') {
      event.preventDefault()
      handleZoom(term, -1)
      return false
    }

    // Ctrl+Shift+0 — reset font
    if (ctrl && shift && event.key === '0') {
      event.preventDefault()
      term.options.fontSize = DEFAULT_FONT_SIZE
      return false
    }

    // Ctrl+Shift+Home — scroll to top
    if (ctrl && shift && event.key === 'Home') {
      event.preventDefault()
      term.scrollToTop()
      return false
    }

    // Ctrl+Shift+End — scroll to bottom
    if (ctrl && shift && event.key === 'End') {
      event.preventDefault()
      term.scrollToBottom()
      return false
    }

    return true
  })

  // Middle-click paste (common on Linux)
  const onMouseDown = (event) => {
    if (event.button !== 1) return
    event.preventDefault()
    void pasteFromClipboard(term)
  }

  container.addEventListener('mousedown', onMouseDown)

  return () => {
    term.attachCustomKeyEventHandler(() => true)
    container.removeEventListener('mousedown', onMouseDown)
  }
}
