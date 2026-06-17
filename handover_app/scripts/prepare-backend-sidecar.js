#!/usr/bin/env node
import { copyFileSync, chmodSync, existsSync, mkdirSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const __dirname = dirname(fileURLToPath(import.meta.url))
const appRoot = resolve(__dirname, '..')
const repoRoot = resolve(appRoot, '..')
const profile = process.argv[2] === 'release' ? 'release' : 'debug'

function sidecarTargetTriple() {
  const archMap = {
    x64: 'x86_64',
    arm64: 'aarch64',
  }
  const arch = archMap[process.arch]
  if (!arch) {
    throw new Error(`Unsupported architecture for Tauri sidecar: ${process.arch}`)
  }

  if (process.platform === 'linux') return `${arch}-unknown-linux-gnu`
  if (process.platform === 'darwin') return `${arch}-apple-darwin`
  if (process.platform === 'win32') return `${arch}-pc-windows-msvc`

  throw new Error(`Unsupported platform for Tauri sidecar: ${process.platform}`)
}

const isWindows = process.platform === 'win32'
const binaryName = `handover-backend${isWindows ? '.exe' : ''}`
const source = join(repoRoot, 'handover_rust', 'target', profile, binaryName)
const targetTriple = sidecarTargetTriple()
const destDir = join(appRoot, 'src-tauri', 'binaries')
const dest = join(
  destDir,
  `handover-backend-${targetTriple}${isWindows ? '.exe' : ''}`,
)

if (!existsSync(source)) {
  const buildHint =
    profile === 'release'
      ? 'cd handover_rust && cargo build --release'
      : 'cd handover_rust && cargo build'
  console.error(`Missing backend binary: ${source}`)
  console.error(`Run: ${buildHint}`)
  process.exit(1)
}

mkdirSync(destDir, { recursive: true })
copyFileSync(source, dest)
if (!isWindows) {
  chmodSync(dest, 0o755)
}

console.log(`Sidecar ready: ${dest}`)
