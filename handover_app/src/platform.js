/** Native folder picker (Tauri). */
export async function pickDirectory() {
  if (!import.meta.env.TAURI_ENV_PLATFORM) {
    throw new Error('Directory picker is only available in the desktop app')
  }
  const { open } = await import('@tauri-apps/plugin-dialog')
  const selected = await open({
    directory: true,
    multiple: false,
    title: 'Select project folder',
  })
  if (selected == null) {
    return null
  }
  return Array.isArray(selected) ? selected[0] : selected
}

/** Backend port injected by Tauri before the React tree mounts. */
export function getBackendPort() {
  if (window.__HANDOVER_BACKEND_PORT__ != null) {
    return String(window.__HANDOVER_BACKEND_PORT__)
  }
  const fromQuery = new URLSearchParams(window.location.search).get('port')
  return fromQuery ?? '8765'
}

/**
 * Per-launch secret the backend requires. Injected by Tauri; for browser-only
 * dev, pass the token the standalone backend prints: `?port=…&token=…`.
 */
export function getBackendToken() {
  if (window.__HANDOVER_BACKEND_TOKEN__ != null) {
    return String(window.__HANDOVER_BACKEND_TOKEN__)
  }
  return new URLSearchParams(window.location.search).get('token') ?? ''
}

export function getBackendBaseUrl() {
  return `http://127.0.0.1:${getBackendPort()}`
}

/** WebSocket URL with the token as a query parameter (browsers can't set WS headers). */
export function getBackendWsUrl(path) {
  const normalized = path.startsWith('/') ? path : `/${path}`
  const sep = normalized.includes('?') ? '&' : '?'
  const token = encodeURIComponent(getBackendToken())
  return `ws://127.0.0.1:${getBackendPort()}${normalized}${sep}token=${token}`
}
