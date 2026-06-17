import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import './index.css'

async function bootstrap() {
  if (import.meta.env.TAURI_ENV_PLATFORM) {
    const { invoke } = await import('@tauri-apps/api/core')
    window.__HANDOVER_BACKEND_PORT__ = String(await invoke('get_backend_port'))
  }

  const { default: App } = await import('./App.jsx')
  createRoot(document.getElementById('root')).render(
    <StrictMode>
      <App />
    </StrictMode>,
  )
}

void bootstrap()
