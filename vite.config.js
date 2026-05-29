import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

// https://vite.dev/config/
export default defineConfig({
  plugins: [react()],
  base: './',
  server: {
    // Bind IPv4 explicitly. Vite otherwise resolves "localhost" to IPv6
    // (::1) on some systems, which breaks the dev:electron wait-on check
    // (tcp:127.0.0.1:5173) and Electron's VITE_DEV_SERVER_URL.
    host: '127.0.0.1',
    port: 5173,
    strictPort: true,
  },
})
