import axios from 'axios'
import { getBackendBaseUrl, getBackendToken } from './platform.js'
import { parseHandoffResult } from './handoffResult.js'

export const BASE_URL = getBackendBaseUrl()

const client = axios.create({
  baseURL: BASE_URL,
  headers: {
    'Content-Type': 'application/json',
    'X-Handover-Token': getBackendToken(),
  },
})

async function request(fn) {
  try {
    const data = await fn()
    return { data, error: null }
  } catch (error) {
    const err = axios.isAxiosError(error)
      ? error.response?.data !== undefined
        ? typeof error.response.data === 'object' && error.response.data !== null
          ? { httpStatus: error.response.status, ...error.response.data }
          : { httpStatus: error.response.status, detail: error.response.data }
        : { message: error.message, httpStatus: error.response?.status }
      : error
    return { data: null, error: err }
  }
}

export async function listProjects() {
  return request(async () => {
    const res = await client.get('/api/projects')
    return res.data
  })
}

export async function createProject(path, sandboxMode = 'docker') {
  return request(async () => {
    const res = await client.post('/api/projects', {
      path,
      sandbox_mode: sandboxMode,
    })
    return res.data
  })
}

export async function activateProject(id) {
  return request(async () => {
    const res = await client.post(`/api/projects/${encodeURIComponent(id)}/activate`)
    return res.data
  })
}

export async function unloadProject(id) {
  return request(async () => {
    const res = await client.post(`/api/projects/${encodeURIComponent(id)}/unload`)
    return res.data
  })
}

/**
 * Opens a terminal. The backend applies the project's effective config
 * (including `.handover/config.yml`); pass `sandboxMode` only for an explicit
 * Native/Docker choice.
 */
export async function startInstance(projectId, sandboxMode) {
  return request(async () => {
    const body = { project_id: projectId }
    if (sandboxMode) body.sandbox_mode = sandboxMode
    const res = await client.post('/api/instances/start', body)
    return res.data
  })
}

export async function getProjectConfig(projectId) {
  return request(async () => {
    const res = await client.get(
      `/api/projects/${encodeURIComponent(projectId)}/config`,
    )
    return res.data
  })
}

export async function saveProjectConfig(projectId, config) {
  return request(async () => {
    const res = await client.put(
      `/api/projects/${encodeURIComponent(projectId)}/config`,
      config,
    )
    return res.data
  })
}

/**
 * Trigger an AI-to-AI handover (backend route: POST /api/handoff).
 * Resolves to `{ ok, kind, message }` (see parseHandoffResult); never throws.
 */
export async function executeHandover(payload) {
  const { data, error } = await request(async () => {
    const res = await client.post('/api/handoff', {
      from_instance_id: payload.from_instance_id,
      to_instance_id: payload.to_instance_id,
      project_id: payload.project_id,
      method: payload.method,
      task_description: payload.task_description,
    })
    return res.data
  })
  return parseHandoffResult(error ?? data)
}

export async function stopInstance(id) {
  return request(async () => {
    const res = await client.delete(`/api/instances/${encodeURIComponent(id)}`)
    return res.data
  })
}

export async function focusInstance(id) {
  return request(async () => {
    const res = await client.post(`/api/instances/${encodeURIComponent(id)}/focus`)
    return res.data
  })
}

export async function getInstanceStats(instanceId) {
  return request(async () => {
    const res = await client.get(
      `/api/instances/${encodeURIComponent(instanceId)}/stats`,
    )
    return res.data
  })
}

export async function listHandoffFiles(projectId) {
  return request(async () => {
    const res = await client.get(
      `/api/projects/${encodeURIComponent(projectId)}/handoffs`,
    )
    return res.data
  })
}

export async function getHandoffFile(projectId, filename) {
  return request(async () => {
    const res = await client.get(
      `/api/projects/${encodeURIComponent(projectId)}/handoffs/${encodeURIComponent(filename)}`,
    )
    return res.data
  })
}

export async function exportHandoffLog(projectId) {
  return request(async () => {
    const res = await client.get(
      `/api/projects/${encodeURIComponent(projectId)}/handoffs/export`,
      { responseType: 'text' },
    )
    return res.data
  })
}

export async function getProjectResources(projectId) {
  return request(async () => {
    const res = await client.get(
      `/api/projects/${encodeURIComponent(projectId)}/resources`,
    )
    return res.data
  })
}

export async function broadcastPrompt(projectId, prompt) {
  return request(async () => {
    const res = await client.post(
      `/api/projects/${encodeURIComponent(projectId)}/broadcast`,
      { prompt },
    )
    return res.data
  })
}

export async function diffHandoffFiles(projectId, from, to) {
  return request(async () => {
    const res = await client.get(
      `/api/projects/${encodeURIComponent(projectId)}/handoffs/diff`,
      { params: { from, to } },
    )
    return res.data
  })
}

export async function gitDiff(projectId, from, to) {
  return request(async () => {
    const res = await client.get(
      `/api/projects/${encodeURIComponent(projectId)}/git-diff`,
      { params: { from, to } },
    )
    return res.data
  })
}
