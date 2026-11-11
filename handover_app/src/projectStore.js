/**
 * The UI's copy of Projects and their Instances, plus which one is focused.
 *
 * Projects arrive from the backend as one shape (`ProjectView`): the
 * Project's fields, its effective `config`, and its open `instances`, each
 * an `Instance` with an `id` and `project_id`. The store trusts that shape;
 * the backend owns defaults and validation.
 *
 * `projectReducer` is pure, so every transition is testable without React.
 */

export const initialProjectState = {
  projects: [],
  focusedProjectId: null,
  focusedInstanceId: null,
}

export const projectActions = {
  /** Full list from GET /api/projects; the server is the truth. */
  loaded: (projects) => ({ type: 'loaded', projects }),
  /** A Project created (or re-opened) by POST /api/projects. */
  projectAdded: (project) => ({ type: 'projectAdded', project }),
  /** Focus a Project and its first Instance (if any). */
  projectFocused: (projectId) => ({ type: 'projectFocused', projectId }),
  /**
   * A new Instance from POST /api/instances/start. It takes focus only if
   * its Project is still the focused one (the user may have moved on while
   * a container was starting; the backend's focus moved with them).
   */
  instanceOpened: (instance) => ({ type: 'instanceOpened', instance }),
  /** Focus an Instance (and the Project it belongs to). */
  instanceFocused: (instanceId) => ({ type: 'instanceFocused', instanceId }),
  /** An Instance closed; focus moves to its neighbour. */
  instanceClosed: (instanceId) => ({ type: 'instanceClosed', instanceId }),
  /** A Project unloaded; focus clears if it was focused. */
  projectClosed: (projectId) => ({ type: 'projectClosed', projectId }),
  /** Config returned by PUT /api/projects/{id}/config. */
  configSaved: (projectId, config) => ({ type: 'configSaved', projectId, config }),
}

export function findProject(state, projectId) {
  return state.projects.find((p) => p.id === projectId) ?? null
}

function projectOfInstance(projects, instanceId) {
  return projects.find((p) => p.instances.some((i) => i.id === instanceId)) ?? null
}

function firstInstanceId(project) {
  return project?.instances[0]?.id ?? null
}

/** Keeps focus pointing at things that still exist. */
function withValidFocus(state) {
  const project = findProject(state, state.focusedProjectId)
  if (!project) {
    return { ...state, focusedProjectId: null, focusedInstanceId: null }
  }
  const instanceStillOpen = project.instances.some(
    (i) => i.id === state.focusedInstanceId,
  )
  return instanceStillOpen
    ? state
    : { ...state, focusedInstanceId: firstInstanceId(project) }
}

function updateProject(state, projectId, update) {
  return {
    ...state,
    projects: state.projects.map((p) => (p.id === projectId ? update(p) : p)),
  }
}

export function projectReducer(state, action) {
  switch (action.type) {
    case 'loaded':
      return withValidFocus({ ...state, projects: action.projects })

    case 'projectAdded': {
      const exists = state.projects.some((p) => p.id === action.project.id)
      return {
        ...state,
        projects: exists
          ? state.projects.map((p) => (p.id === action.project.id ? action.project : p))
          : [...state.projects, action.project],
      }
    }

    case 'projectFocused': {
      const project = findProject(state, action.projectId)
      if (!project) return state
      return {
        ...state,
        focusedProjectId: project.id,
        focusedInstanceId: firstInstanceId(project),
      }
    }

    case 'instanceOpened': {
      const { instance } = action
      if (!findProject(state, instance.project_id)) return state
      const next = updateProject(state, instance.project_id, (p) =>
        p.instances.some((i) => i.id === instance.id)
          ? p
          : { ...p, instances: [...p.instances, instance] },
      )
      return state.focusedProjectId === instance.project_id
        ? { ...next, focusedInstanceId: instance.id }
        : next
    }

    case 'instanceFocused': {
      const project = projectOfInstance(state.projects, action.instanceId)
      if (!project) return state
      return {
        ...state,
        focusedProjectId: project.id,
        focusedInstanceId: action.instanceId,
      }
    }

    case 'instanceClosed': {
      const project = projectOfInstance(state.projects, action.instanceId)
      if (!project) return state
      const index = project.instances.findIndex((i) => i.id === action.instanceId)
      const remaining = project.instances.filter((i) => i.id !== action.instanceId)
      const next = updateProject(state, project.id, (p) => ({ ...p, instances: remaining }))
      if (state.focusedInstanceId !== action.instanceId) return next
      const neighbour = remaining[index] ?? remaining[index - 1] ?? null
      return { ...next, focusedInstanceId: neighbour?.id ?? null }
    }

    case 'projectClosed':
      return withValidFocus({
        ...state,
        projects: state.projects.filter((p) => p.id !== action.projectId),
      })

    case 'configSaved':
      return updateProject(state, action.projectId, (p) => ({
        ...p,
        name: action.config.project_name || p.name,
        config: action.config,
      }))

    default:
      return state
  }
}

/** Every open Instance across all Projects, in sidebar order. */
export function allInstances(state) {
  return state.projects.flatMap((p) => p.instances)
}
