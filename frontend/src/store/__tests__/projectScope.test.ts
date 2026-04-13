import { useStore } from '@/store'
import { buildRunsScopeKey } from '@/state/runsSessionScope'
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
    workingDir: DEFAULT_WORKING_DIRECTORY,
    projectRegistry: {},
    projectSessionsByPath: {},
    projectRegistrationError: null,
    recentProjectPaths: [],
    graphAttrs: {},
    graphAttrErrors: {},
    graphAttrsUserEditVersion: 0,
    editorGraphSettingsPanelOpenByFlow: {},
    editorShowAdvancedGraphAttrsByFlow: {},
    editorLaunchInputDraftsByFlow: {},
    editorLaunchInputDraftErrorByFlow: {},
    editorNodeInspectorSessionsByNodeId: {},
  }))
}

const buildRunRecord = (runId: string, projectPath: string, flowName: string) => ({
  run_id: runId,
  flow_name: flowName,
  status: 'completed' as const,
  outcome: 'success' as const,
  outcome_reason_code: null,
  outcome_reason_message: null,
  working_directory: `${projectPath}/workspace`,
  project_path: projectPath,
  git_branch: 'main',
  git_commit: 'abcdef0',
  spec_id: null,
  plan_id: null,
  model: 'gpt-5.4',
  started_at: '2026-03-22T00:00:00Z',
  ended_at: '2026-03-22T00:05:00Z',
  last_error: null,
  token_usage: 144,
  current_node: 'done',
  continued_from_run_id: null,
  continued_from_node: null,
  continued_from_flow_mode: null,
  continued_from_flow_name: null,
})

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
  })

  it('foregrounds the remembered project-scoped selected run when switching active projects', () => {
    const store = useStore.getState()
    store.registerProject('/tmp/project-a')
    store.registerProject('/tmp/project-b')

    store.updateRunDetailSession('run-a', {
      summaryRecord: buildRunRecord('run-a', '/tmp/project-a', 'project-a.dot'),
      completedNodesSnapshot: ['plan'],
      statusFetchedAtMs: 101,
    })
    store.updateRunDetailSession('run-b', {
      summaryRecord: buildRunRecord('run-b', '/tmp/project-b', 'project-b.dot'),
      completedNodesSnapshot: ['review'],
      statusFetchedAtMs: 202,
    })
    store.setRunsSelectedRunIdForScope(buildRunsScopeKey('active', '/tmp/project-a'), 'run-a')
    store.setRunsSelectedRunIdForScope(buildRunsScopeKey('active', '/tmp/project-b'), 'run-b')

    store.setActiveProjectPath('/tmp/project-b')
    let next = useStore.getState()
    expect(next.selectedRunId).toBe('run-b')
    expect(next.selectedRunRecord?.flow_name).toBe('project-b.dot')
    expect(next.selectedRunCompletedNodes).toEqual(['review'])
    expect(next.selectedRunStatusFetchedAtMs).toBe(202)

    store.setActiveProjectPath('/tmp/project-a')
    next = useStore.getState()
    expect(next.selectedRunId).toBe('run-a')
    expect(next.selectedRunRecord?.flow_name).toBe('project-a.dot')
    expect(next.selectedRunCompletedNodes).toEqual(['plan'])
    expect(next.selectedRunStatusFetchedAtMs).toBe(101)
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
    store.updateRunDetailSession('run-a', {
      summaryRecord: buildRunRecord('run-a', '/tmp/project-a', 'project-a.dot'),
      completedNodesSnapshot: ['plan'],
      statusFetchedAtMs: 101,
    })
    store.updateRunDetailSession('run-b', {
      summaryRecord: buildRunRecord('run-b', '/tmp/project-b', 'project-b.dot'),
      completedNodesSnapshot: ['review'],
      statusFetchedAtMs: 202,
    })
    store.setRunsSelectedRunIdForScope(buildRunsScopeKey('active', '/tmp/project-a'), 'run-a')
    store.setRunsSelectedRunIdForScope(buildRunsScopeKey('active', '/tmp/project-b'), 'run-b')

    store.removeProject('/tmp/project-a', '/tmp/project-b')

    const next = useStore.getState()
    expect(next.projectRegistry['/tmp/project-a']).toBeUndefined()
    expect(next.activeProjectPath).toBe('/tmp/project-b')
    expect(next.runsListSession.selectedRunIdByScopeKey[buildRunsScopeKey('active', '/tmp/project-a')]).toBeUndefined()
    expect(next.runsListSession.selectedRunIdByScopeKey[buildRunsScopeKey('active', '/tmp/project-b')]).toBe('run-b')
    expect(next.runDetailSessionsByRunId['run-a']).toBeUndefined()
    expect(next.runDetailSessionsByRunId['run-b']?.summaryRecord?.run_id).toBe('run-b')
  })
})
