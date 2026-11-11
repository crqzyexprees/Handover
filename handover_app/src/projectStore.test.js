import { describe, expect, it } from 'vitest'
import {
  allInstances,
  initialProjectState,
  projectActions as a,
  projectReducer,
} from './projectStore.js'

const config = (over = {}) => ({
  project_name: 'p',
  sandbox_mode: 'docker',
  mem_limit: '2g',
  handoff_method: 'summary',
  handoff_template: 'generic',
  custom_env_vars: {},
  ...over,
})
const inst = (id, projectId) => ({ id, project_id: projectId, sandbox_mode: 'native' })
const project = (id, instanceIds = []) => ({
  id,
  path: `/work/${id}`,
  name: id,
  sandbox_mode: 'docker',
  state: 'active',
  config: config({ project_name: id }),
  instances: instanceIds.map((i) => inst(i, id)),
})

const run = (...actions) => actions.reduce(projectReducer, initialProjectState)

describe('projectReducer', () => {
  it('focusing a project focuses its first instance', () => {
    const s = run(a.loaded([project('p1', ['i1', 'i2'])]), a.projectFocused('p1'))
    expect(s.focusedProjectId).toBe('p1')
    expect(s.focusedInstanceId).toBe('i1')
  })

  it('an opened instance in the focused project takes focus', () => {
    const s = run(
      a.loaded([project('p1', ['i1'])]),
      a.projectFocused('p1'),
      a.instanceOpened(inst('i9', 'p1')),
    )
    expect(s.projects[0].instances.map((i) => i.id)).toEqual(['i1', 'i9'])
    expect(s.focusedInstanceId).toBe('i9')
  })

  it('an instance that finishes starting after the user moved on does not steal focus', () => {
    const s = run(
      a.loaded([project('p1'), project('p2', ['i2'])]),
      a.projectFocused('p1'),
      a.projectFocused('p2'),
      a.instanceOpened(inst('i9', 'p1')),
    )
    expect(s.projects[0].instances.map((i) => i.id)).toEqual(['i9'])
    expect(s.focusedProjectId).toBe('p2')
    expect(s.focusedInstanceId).toBe('i2')
  })

  it('opening the same instance twice adds it once', () => {
    const s = run(
      a.loaded([project('p1')]),
      a.instanceOpened(inst('i1', 'p1')),
      a.instanceOpened(inst('i1', 'p1')),
    )
    expect(s.projects[0].instances).toHaveLength(1)
  })

  it('closing the focused instance focuses the next, then the previous', () => {
    let s = run(a.loaded([project('p1', ['i1', 'i2', 'i3'])]), a.instanceFocused('i2'))
    s = projectReducer(s, a.instanceClosed('i2'))
    expect(s.focusedInstanceId).toBe('i3')
    s = projectReducer(s, a.instanceClosed('i3'))
    expect(s.focusedInstanceId).toBe('i1')
    s = projectReducer(s, a.instanceClosed('i1'))
    expect(s.focusedInstanceId).toBeNull()
    expect(s.focusedProjectId).toBe('p1')
  })

  it('closing an unfocused instance keeps focus', () => {
    const s = run(
      a.loaded([project('p1', ['i1', 'i2'])]),
      a.instanceFocused('i1'),
      a.instanceClosed('i2'),
    )
    expect(s.focusedInstanceId).toBe('i1')
  })

  it('focusing an instance in another project switches project', () => {
    const s = run(
      a.loaded([project('p1', ['i1']), project('p2', ['i2'])]),
      a.projectFocused('p1'),
      a.instanceFocused('i2'),
    )
    expect(s.focusedProjectId).toBe('p2')
  })

  it('closing the focused project clears focus', () => {
    const s = run(
      a.loaded([project('p1', ['i1']), project('p2')]),
      a.projectFocused('p1'),
      a.projectClosed('p1'),
    )
    expect(s.projects.map((p) => p.id)).toEqual(['p2'])
    expect(s.focusedProjectId).toBeNull()
    expect(s.focusedInstanceId).toBeNull()
  })

  it('a reload drops focus on instances that no longer exist', () => {
    const s = run(
      a.loaded([project('p1', ['i1', 'i2'])]),
      a.instanceFocused('i2'),
      a.loaded([project('p1', ['i1'])]),
    )
    expect(s.focusedInstanceId).toBe('i1')
  })

  it('a re-added project replaces the old entry', () => {
    const s = run(a.loaded([project('p1', ['i1'])]), a.projectAdded(project('p1')))
    expect(s.projects).toHaveLength(1)
    expect(s.projects[0].instances).toEqual([])
  })

  it('saved config becomes the project config and name', () => {
    const saved = config({ project_name: 'Renamed', handoff_method: 'git' })
    const s = run(a.loaded([project('p1')]), a.configSaved('p1', saved))
    expect(s.projects[0].config).toEqual(saved)
    expect(s.projects[0].name).toBe('Renamed')
  })

  it('lists every instance across projects', () => {
    const s = run(a.loaded([project('p1', ['i1']), project('p2', ['i2', 'i3'])]))
    expect(allInstances(s).map((i) => i.id)).toEqual(['i1', 'i2', 'i3'])
  })
})
