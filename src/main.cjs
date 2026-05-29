const { spawn } = require('node:child_process')
const fs = require('node:fs')
const net = require('node:net')
const path = require('node:path')

const { app, BrowserWindow, dialog, ipcMain, Menu } = require('electron')

const BACKEND_HOST = '127.0.0.1'

let backendProcess = null
// A free TCP port chosen at startup that the backend binds to and the renderer
// talks to. Avoids collisions when 8765 is taken or multiple instances run.
let backendPort = null

// Ask the OS for an unused loopback port (bind to :0, read it, release it).
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

  const backendDir = path.join(__dirname, '../backend')
  // Invoke the venv's python with `-m uvicorn` rather than the `uvicorn`
  // console script: python is a (relative) symlink that keeps working if the
  // venv is moved/renamed, whereas the console script bakes in an absolute
  // shebang path that breaks on rename.
  const python = path.join(backendDir, 'venv/bin/python')
  if (!fs.existsSync(python)) {
    throw new Error(
      `Dev backend venv not found: ${python}. Create the venv and install dependencies.`,
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

function startBackend() {
  const { command, args, options } = getBackendSpawnOptions()
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

  // Hand the chosen backend port to the renderer via a `?port=` query param,
  // which api.js / TerminalView.jsx read from window.location.search.
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
