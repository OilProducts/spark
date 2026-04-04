import { useStore } from '@/store'
import { beforeEach, describe, expect, it } from 'vitest'

const DEFAULT_WORKING_DIRECTORY = './test-app'

const resetStore = () => {
  useStore.setState((state) => ({
    ...state,
    viewMode: 'projects',
    activeProjectPath: null,
    activeFlow: null,
    executionFlow: null,
    selectedRunId: null,
    selectedRunRecord: null,
    selectedRunCompletedNodes: [],
    selectedRunStatusSync: 'idle',
    selectedRunStatusError: null,
    selectedRunStatusFetchedAtMs: null,
    runRecordOverrides: {},
    workingDir: DEFAULT_WORKING_DIRECTORY,
    projectRegistry: {},
    projectSessionsByPath: {},
    projectRegistrationError: null,
    recentProjectPaths: [],
    graphAttrs: {},
    graphAttrErrors: {},
    graphAttrsUserEditVersion: 0,
  }))
}

describe('project scope store behavior', () => {
  beforeEach(() => {
    resetStore()
  })

  it('allows selecting editor without an active project', () => {
    const store = useStore.getState()
    store.setViewMode('editor')

    expect(useStore.getState().viewMode).toBe('editor')
  })

  it('rejects non-absolute project paths', () => {
    const result = useStore.getState().registerProject('relative/path')

    expect(result.ok).toBe(false)
    expect(result.error).toBe('Project directory path must be absolute.')
    expect(useStore.getState().projectRegistrationError).toBe('Project directory path must be absolute.')
  })

  it('normalizes and registers an absolute project path', () => {
    const result = useStore.getState().registerProject(' /tmp/demo//project/./subdir/.. ')

    expect(result.ok).toBe(true)
    expect(result.normalizedPath).toBe('/tmp/demo/project')
    expect(useStore.getState().projectRegistry['/tmp/demo/project']).toBeDefined()
    expect(useStore.getState().activeProjectPath).toBe('/tmp/demo/project')
  })

  it('prevents duplicate project registrations', () => {
    useStore.getState().registerProject('/tmp/demo/project')

    const duplicateResult = useStore.getState().registerProject('/tmp/demo/project')

    expect(duplicateResult.ok).toBe(false)
    expect(duplicateResult.error).toBe('Project already registered: /tmp/demo/project')
  })

  it('resets run state but preserves selected flows when switching active projects', () => {
    const store = useStore.getState()
    store.registerProject('/tmp/project-a')
    store.registerProject('/tmp/project-b')

    store.setRuntimeStatus('running')
    store.setSelectedRunId('run-a')
    store.setActiveFlow('preferred.dot')
    store.setExecutionFlow('run-flow-a.dot')
    store.addLog({ time: '12:00', msg: 'running', type: 'info' })
    store.setSelectedNodeId('node-a')
    store.setDiagnostics([
      {
        rule_id: 'test',
        severity: 'warning',
        message: 'diag',
      },
    ])
    store.setGraphAttrs({ goal: 'A' })
    store.markSaveSuccess()
    store.setActiveProjectPath('/tmp/project-b')

    const next = useStore.getState()
    expect(next.runtimeStatus).toBe('idle')
    expect(next.selectedRunId).toBeNull()
    expect(next.activeFlow).toBe('preferred.dot')
    expect(next.executionFlow).toBe('run-flow-a.dot')
    expect(next.logs).toEqual([])
  })

  it('tracks user graph attr edits separately from hydrated replacements', () => {
    const store = useStore.getState()

    store.replaceGraphAttrs({ goal: 'Hydrated goal' })
    let next = useStore.getState()
    expect(next.graphAttrs).toEqual({ goal: 'Hydrated goal' })
    expect(next.graphAttrsUserEditVersion).toBe(0)

    store.updateGraphAttr('goal', 'Edited goal')
    next = useStore.getState()
    expect(next.graphAttrs.goal).toBe('Edited goal')
    expect(next.graphAttrsUserEditVersion).toBe(1)

    store.setGraphAttrs({
      ...next.graphAttrs,
      retry_target: 'retry-node',
    })
    next = useStore.getState()
    expect(next.graphAttrs.retry_target).toBe('retry-node')
    expect(next.graphAttrsUserEditVersion).toBe(2)
  })

  it('does not persist run selection into project-scoped workspace state', () => {
    const store = useStore.getState()
    store.registerProject('/tmp/project-a')

    store.setSelectedRunId('run-a')

    expect(useStore.getState().selectedRunId).toBe('run-a')
    expect(useStore.getState().projectSessionsByPath['/tmp/project-a']).toBeDefined()
  })

  it('keeps the inspected execution flow separate from the current editor flow', () => {
    const store = useStore.getState()
    store.registerProject('/tmp/project-a')

    store.setActiveFlow('preferred.dot')
    store.setExecutionFlow('run-opened.dot')

    const next = useStore.getState()
    expect(next.activeFlow).toBe('preferred.dot')
    expect(next.executionFlow).toBe('run-opened.dot')
    expect(next.projectSessionsByPath['/tmp/project-a']?.activeFlow).toBeUndefined()
  })

  it('hydrates backend project metadata without deriving a project flow preference', () => {
    const store = useStore.getState()
    store.hydrateProjectRegistry([
      {
        directoryPath: '/tmp/project-a',
        isFavorite: false,
        lastAccessedAt: null,
        activeConversationId: null,
      },
    ])

    const next = useStore.getState()
    expect(next.projectRegistry['/tmp/project-a']).toEqual({
      directoryPath: '/tmp/project-a',
      isFavorite: false,
      lastAccessedAt: null,
    })
    expect(next.projectSessionsByPath['/tmp/project-a']?.activeFlow).toBeUndefined()
    expect(next.activeFlow).toBeNull()
  })

  it('falls back to another registered project when removing the active project', () => {
    const store = useStore.getState()
    store.registerProject('/tmp/project-a')
    store.registerProject('/tmp/project-b')
    store.setActiveProjectPath('/tmp/project-a')

    store.removeProject('/tmp/project-a', '/tmp/project-b')

    const next = useStore.getState()
    expect(next.projectRegistry['/tmp/project-a']).toBeUndefined()
    expect(next.activeProjectPath).toBe('/tmp/project-b')
  })
})
