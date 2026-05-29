import axios from 'axios'

const urlParams = new URLSearchParams(window.location.search)
const port = urlParams.get('port') || '8765'
export const BASE_URL = `http://127.0.0.1:${port}`

const client = axios.create({
  baseURL: BASE_URL,
  headers: { 'Content-Type': 'application/json' },
})

async function request(fn) {
  try {
    const data = await fn()
    return { data, error: null }
  } catch (error) {
    const err = axios.isAxiosError(error)
      ? error.response?.data !== undefined
        ? typeof error.response.data === 'object' && error.response.data !== null
          ? { status: error.response.status, ...error.response.data }
          : { status: error.response.status, detail: error.response.data }
        : { message: error.message, status: error.response?.status }
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

export async function startInstance(
  projectId,
  sandboxMode = 'docker',
  memLimit = '2g',
) {
  return request(async () => {
    const res = await client.post('/api/instances/start', {
      project_id: projectId,
      sandbox_mode: sandboxMode,
      mem_limit: memLimit,
    })
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

/** Trigger an AI-to-AI handover (backend route: POST /api/handoff). */
export async function executeHandover(payload) {
  return request(async () => {
    const res = await client.post('/api/handoff', {
      from_instance_id: payload.from_instance_id,
      to_instance_id: payload.to_instance_id,
      project_id: payload.project_id,
      method: payload.method,
      task_description: payload.task_description,
    })
    return res.data
  })
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
