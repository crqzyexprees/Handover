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

export function getBackendBaseUrl() {
  return `http://127.0.0.1:${getBackendPort()}`
}

export function getBackendWsUrl(path) {
  const normalized = path.startsWith('/') ? path : `/${path}`
  return `ws://127.0.0.1:${getBackendPort()}${normalized}`
}
