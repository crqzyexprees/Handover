const { spawn } = require('node:child_process')
const fs = require('node:fs')
const net = require('node:net')
const path = require('node:path')

const { app, BrowserWindow, dialog, ipcMain, Menu } = require('electron')

const BACKEND_HOST = '127.0.0.1'

let backendProcess = null
let backendPort = null

function getFreePort() {
  return new Promise((resolve, reject) => {
    const srv = net.createServer()
    srv.unref()
    srv.on('error', reject)
    srv.listen(0, BACKEND_HOST, () => {
      const { port } = srv.address()
      srv.close(() => resolve(port))
    })
  })
}

ipcMain.handle('pick-directory', async () => {
  const result = await dialog.showOpenDialog({
    properties: ['openDirectory'],
    title: 'Select project folder',
    buttonLabel: 'Select folder',
  })
  if (result.canceled || !result.filePaths?.length) {
    return null
  }
  return result.filePaths[0]
})

function getRustBackendSpawnOptions() {
  const rustRoot = path.join(__dirname, '../../handover_rust')
  const releaseBin = path.join(rustRoot, 'target/release/handover-backend')
  const debugBin = path.join(rustRoot, 'target/debug/handover-backend')
  const binary = fs.existsSync(releaseBin)
    ? releaseBin
    : fs.existsSync(debugBin)
      ? debugBin
      : null

  if (binary) {
    return {
      command: binary,
      args: ['--host', BACKEND_HOST, '--port', String(backendPort)],
      options: {},
    }
  }

  const manifest = path.join(rustRoot, 'Cargo.toml')
  if (!fs.existsSync(manifest)) {
    throw new Error(
      `Rust backend not found. Build it with: cd handover_rust && cargo build`,
    )
  }

  return {
    command: 'cargo',
    args: [
      'run',
      '--manifest-path',
      manifest,
      '--',
      '--host',
      BACKEND_HOST,
      '--port',
      String(backendPort),
    ],
    options: { cwd: rustRoot },
  }
}

function getPythonBackendSpawnOptions() {
  const backendDir = path.join(__dirname, '../../handover_python')
  const python = path.join(backendDir, 'venv/bin/python')
  if (!fs.existsSync(python)) {
    throw new Error(
      `Python venv not found at ${python}. ` +
        'Create it: cd handover_python && python -m venv venv && ./venv/bin/pip install -r requirements.txt',
    )
  }
  return {
    command: python,
    args: [
      '-m',
      'uvicorn',
      'main:app',
      '--host',
      BACKEND_HOST,
      '--port',
      String(backendPort),
    ],
    options: { cwd: backendDir },
  }
}

function getBackendSpawnOptions() {
  if (app.isPackaged) {
    const executable = path.join(
      process.resourcesPath,
      'backend',
      'handover-backend',
    )
    if (!fs.existsSync(executable)) {
      throw new Error(`Backend executable not found: ${executable}`)
    }
    return {
      command: executable,
      args: ['--host', BACKEND_HOST, '--port', String(backendPort)],
      options: {},
    }
  }

  if (process.env.HANDOVER_BACKEND === 'rust') {
    return getRustBackendSpawnOptions()
  }

  return getPythonBackendSpawnOptions()
}

async function waitForBackend(port, timeoutMs = 120_000) {
  const url = `http://${BACKEND_HOST}:${port}/`
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    try {
      const res = await fetch(url)
      if (res.ok) {
        return
      }
    } catch {
      // Backend still starting
    }
    await new Promise((resolve) => setTimeout(resolve, 250))
  }
  throw new Error(
    `Backend did not respond on ${url} within ${timeoutMs / 1000}s. ` +
      'Ensure the selected backend is set up (see README.md).',
  )
}

function startBackend() {
  const { command, args, options } = getBackendSpawnOptions()
  console.log('[backend] starting:', command, args.join(' '))
  backendProcess = spawn(command, args, {
    ...options,
    env: { ...process.env },
    stdio: 'inherit',
  })

  backendProcess.on('error', (err) => {
    console.error('[backend] failed to start:', err)
  })

  backendProcess.on('exit', (code, signal) => {
    if (code !== 0 && code != null) {
      console.error(`[backend] exited with code ${code}`)
    }
    if (signal) {
      console.error(`[backend] killed by signal ${signal}`)
    }
    backendProcess = null
  })
}

function stopBackend() {
  if (backendProcess && !backendProcess.killed) {
    backendProcess.kill()
    backendProcess = null
  }
}

function createWindow() {
  const win = new BrowserWindow({
    width: 1200,
    height: 800,
    webPreferences: {
      preload: path.join(__dirname, 'preload.cjs'),
      nodeIntegration: false,
      contextIsolation: true,
    },
  })

  const devUrl = process.env.VITE_DEV_SERVER_URL
  if (devUrl) {
    const url = new URL(devUrl)
    url.searchParams.set('port', String(backendPort))
    win.loadURL(url.toString())
  } else {
    win.loadFile(path.join(__dirname, '../dist/index.html'), {
      query: { port: String(backendPort) },
    })
  }
}

app.whenReady().then(async () => {
  Menu.setApplicationMenu(null)
  try {
    backendPort = await getFreePort()
    startBackend()
    await waitForBackend(backendPort)
  } catch (err) {
    console.error(err)
    dialog.showErrorBox(
      'Handover backend failed to start',
      err instanceof Error ? err.message : String(err),
    )
    app.quit()
    return
  }
  createWindow()
})

app.on('before-quit', () => {
  stopBackend()
})

app.on('window-all-closed', () => {
  stopBackend()
  if (process.platform !== 'darwin') {
    app.quit()
  }
})

app.on('activate', () => {
  if (BrowserWindow.getAllWindows().length === 0) {
    createWindow()
  }
})
