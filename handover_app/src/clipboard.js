/** Clipboard helpers — Tauri plugin in desktop app, DOM fallbacks elsewhere. */

function copyViaExecCommand(text) {
  const textarea = document.createElement('textarea')
  textarea.value = text
  textarea.setAttribute('readonly', '')
  textarea.style.position = 'fixed'
  textarea.style.top = '0'
  textarea.style.left = '0'
  textarea.style.opacity = '0'
  document.body.appendChild(textarea)
  textarea.focus()
  textarea.select()
  const ok = document.execCommand('copy')
  document.body.removeChild(textarea)
  return ok
}

export async function writeClipboard(text) {
  if (typeof text !== 'string' || text.length === 0) return false

  if (import.meta.env.TAURI_ENV_PLATFORM) {
    try {
      const { writeText } = await import('@tauri-apps/plugin-clipboard-manager')
      await writeText(text)
      return true
    } catch {
      // fall through to web APIs
    }
  }

  try {
    if (navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(text)
      return true
    }
  } catch {
    // fall through
  }

  return copyViaExecCommand(text)
}

export async function readClipboard() {
  if (import.meta.env.TAURI_ENV_PLATFORM) {
    try {
      const { readText } = await import('@tauri-apps/plugin-clipboard-manager')
      return (await readText()) ?? ''
    } catch {
      // fall through
    }
  }

  try {
    if (navigator.clipboard?.readText) {
      return (await navigator.clipboard.readText()) ?? ''
    }
  } catch {
    // fall through
  }

  return ''
}
