import { selectSelectedRunId, selectSelectedRunSession } from '@/state/runsSessionSelectors'
import { useStore } from '@/store'
import { applyConversationPageToCache, type ProjectConversationCacheState } from '@/features/projects/model/projectsHomeState'
import { parseConversationPageResponse } from '@/lib/workspaceClient'
import { conversationPage } from '@/test/conversationWire'
import { beforeEach, describe, expect, it } from 'vitest'

const DEFAULT_WORKING_DIRECTORY = './test-app'

const resetStore = () => {
  {
useStore.setState({...useStore.getState(),
viewMode: 'projects',
activeProjectPath: null,
activeFlow: null,
workingDir: DEFAULT_WORKING_DIRECTORY,
projectRegistry: {},
projectSessionsByPath: {},
projectRegistrationError: null,
recentProjectPaths: [],
flowMetadata: {},
flowMetadataErrors: {},
flowMetadataUserEditVersion: 0,
graphAttrs: {},
graphAttrErrors: {},
graphAttrsUserEditVersion: 0,
editorGraphSettingsPanelOpenByFlow: {},
editorShowAdvancedFlowMetadataByFlow: {},
editorShowAdvancedGraphAttrsByFlow: {},
editorLaunchInputDraftsByFlow: {},
editorLaunchInputDraftErrorByFlow: {},
editorNodeInspectorSessionsByNodeId: {},
homeConversationCache: {
      conversationsById: {},
      summariesByProjectPath: {},
    },
homeThreadSummariesStatusByProjectPath: {},
homeThreadSummariesErrorByProjectPath: {},
homeProjectSessionsByPath: {},
homeConversationSessionsById: {},
homeProjectGitMetadataByPath: {}});
useStore.getState().setRunsSelectedRunId(null);
}
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

const applyConversationWindow = (
  cache: ProjectConversationCacheState,
  conversationId: string,
  projectPath: string,
) => applyConversationPageToCache(
  cache,
  parseConversationPageResponse(conversationPage(buildConversationSnapshot(conversationId, projectPath))),
  'window',
)

const buildConversationSnapshot = (
  conversationId: string,
  projectPath: string,
) => ({
  revision: 1,
  conversation_id: conversationId,
  conversation_handle: '',
  project_path: projectPath,
  chat_mode: 'chat',
  title: `Thread ${conversationId}`,
  created_at: '2026-03-22T00:00:00Z',
  updated_at: '2026-03-22T00:01:00Z',
  turns: [
    {
      id: `${conversationId}-turn-user`,
      role: 'user',
      content: 'Hello',
      timestamp: '2026-03-22T00:00:00Z',
      status: 'complete',
      kind: 'message',
      artifact_id: null,
    },
  ],
  segments: [],
  flow_run_requests: [],
  flow_launches: [],
  proposed_plans: [],
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

  it('keeps the run selection but clears the open flow when the chat project changes', () => {
    const store = useStore.getState()
    store.registerProject('/tmp/project-a')
    store.registerProject('/tmp/project-b')

    store.setRunsSelectedRunId('run-a')
    store.setActiveFlow('preferred.dot')
    store.setActiveProjectPath('/tmp/project-a')
    expect(useStore.getState().activeFlow).toBe('preferred.dot')
    store.setActiveProjectPath('/tmp/project-b')

    // Flows are per project: the same name in project-b is a different flow.
    const next = useStore.getState()
    expect(selectSelectedRunId(next)).toBe('run-a')
    expect(next.activeFlow).toBeNull()
  })

  it('shows the chat, not a project page, when a chat project is chosen', () => {
    const store = useStore.getState()
    store.registerProject('/tmp/project-a')
    store.openProjectPage('/tmp/project-a')
    expect(useStore.getState().projectPagePath).toBe('/tmp/project-a')
    store.setActiveProjectPath('/tmp/project-a')
    expect(useStore.getState().projectPagePath).toBeNull()
  })

  it('tracks user FlowDefinition metadata edits separately from hydrated replacements', () => {
    const store = useStore.getState()

    store.replaceFlowMetadata({ goal: 'Hydrated goal' })
    let next = useStore.getState()
    expect(next.flowMetadata).toEqual({ goal: 'Hydrated goal' })
    expect(next.flowMetadataUserEditVersion).toBe(0)

    store.updateFlowMetadata('goal', 'Edited goal')
    next = useStore.getState()
    expect(next.flowMetadata.goal).toBe('Edited goal')
    expect(next.flowMetadataUserEditVersion).toBe(1)

    store.setFlowMetadata({
      ...next.flowMetadata,
      owner: 'workflow-team',
    })
    next = useStore.getState()
    expect(next.flowMetadata.owner).toBe('workflow-team')
    expect(next.flowMetadataUserEditVersion).toBe(2)
  })

  it('does not persist run selection into project-scoped workspace state', () => {
    const store = useStore.getState()
    store.registerProject('/tmp/project-a')

    store.setRunsSelectedRunId('run-a')

    expect(selectSelectedRunId(useStore.getState())).toBe('run-a')
    expect(useStore.getState().projectSessionsByPath['/tmp/project-a']).toBeDefined()
  })

  it('keeps the editor flow selection out of the project session record', () => {
    const store = useStore.getState()
    store.registerProject('/tmp/project-a')

    store.setActiveFlow('preferred.dot')

    const next = useStore.getState()
    expect(next.activeFlow).toBe('preferred.dot')
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

  it('hydrates into the default Home project only when no registered project is active', () => {
    const projects = [
      { directoryPath: '/tmp/project-a', isFavorite: false, lastAccessedAt: null },
      { directoryPath: '/home/user', displayName: 'Home', isDefault: true, isFavorite: false, lastAccessedAt: null },
    ]
    useStore.getState().hydrateProjectRegistry(projects)
    expect(useStore.getState().activeProjectPath).toBe('/home/user')
    expect(useStore.getState().projectRegistry['/home/user']?.displayName).toBe('Home')

    useStore.getState().setActiveProjectPath('/tmp/project-a')
    useStore.getState().hydrateProjectRegistry(projects)
    expect(useStore.getState().activeProjectPath).toBe('/tmp/project-a')
  })

  it('falls back to another registered project when removing the active project', () => {
    const store = useStore.getState()
    store.registerProject('/tmp/project-a')
    store.registerProject('/tmp/project-b')
    store.setActiveProjectPath('/tmp/project-a')
    store.setRunsSelectedRunId('run-a')
    store.setRunsSelectedRunId('run-b')
    store.updateRunDetailSession('run-a', {
      record: buildRunRecord('run-a', '/tmp/project-a', 'project-a.dot'),
      completedNodesSnapshot: ['plan'],
      statusFetchedAtMs: 101,
    })
    store.updateRunDetailSession('run-b', {
      record: buildRunRecord('run-b', '/tmp/project-b', 'project-b.dot'),
      completedNodesSnapshot: ['review'],
      statusFetchedAtMs: 202,
    })
    store.setRunsSelectedRunId('run-a')
    store.setRunsSelectedRunId('run-b')

    store.removeProject('/tmp/project-a', '/tmp/project-b')

    const next = useStore.getState()
    expect(next.projectRegistry['/tmp/project-a']).toBeUndefined()
    expect(next.activeProjectPath).toBe('/tmp/project-b')
    expect(selectSelectedRunId(next)).toBe('run-b')
    expect(next.runDetailSessionsByRunId['run-a']).toBeUndefined()
    expect(next.runDetailSessionsByRunId['run-b']?.record?.run_id).toBe('run-b')
  })

  it('keeps the runs and run selection of a project registered inside a removed one', () => {
    const store = useStore.getState()
    store.registerProject('/tmp/work')
    store.registerProject('/tmp/work/child')
    store.updateRunsListSession({ runs: [buildRunRecord('run-parent', '/tmp/work', 'a.dot'), buildRunRecord('run-child', '/tmp/work/child', 'b.dot')] })
    store.updateRunDetailSession('run-child', { record: buildRunRecord('run-child', '/tmp/work/child', 'b.dot') })
    store.setRunsSelectedRunId('run-child')

    store.removeProject('/tmp/work', '/tmp/work/child')

    const next = useStore.getState()
    expect(next.runsListSession.runs.map((run) => run.run_id)).toEqual(['run-child'])
    expect(selectSelectedRunId(next)).toBe('run-child')
    expect(next.runDetailSessionsByRunId['run-child']).toBeDefined()
  })

  it('keeps another project\'s run that worked inside the removed project, and drops the removed project\'s own runs', () => {
    const store = useStore.getState()
    store.registerProject('/tmp/project-a')
    store.registerProject('/tmp/project-b')
    const crossRun = { ...buildRunRecord('run-b', '/tmp/project-b', 'b.dot'), working_directory: '/tmp/project-a/scratch' }
    const ownRun = { ...buildRunRecord('run-a', '/tmp/project-a', 'a.dot'), working_directory: '/tmp/elsewhere/worktree' }
    const unownedRun = { ...buildRunRecord('run-loose', '/tmp/project-a', 'loose.dot'), project_path: null }
    store.updateRunsListSession({ runs: [crossRun, ownRun, unownedRun] })
    store.updateRunDetailSession('run-b', { record: crossRun })
    store.updateRunDetailSession('run-a', { record: ownRun })
    store.setRunsSelectedRunId('run-b')

    store.removeProject('/tmp/project-a', '/tmp/project-b')

    const next = useStore.getState()
    expect(next.runsListSession.runs.map((run) => run.run_id)).toEqual(['run-b'])
    expect(selectSelectedRunId(next)).toBe('run-b')
    expect(next.runDetailSessionsByRunId['run-b']).toBeDefined()
    expect(next.runDetailSessionsByRunId['run-a']).toBeUndefined()
  })

  it('keeps a run started in an unregistered child folder when its parent C:/work is removed', () => {
    const store = useStore.getState()
    store.registerProject('C:\\work')
    store.registerProject('C:\\other')
    // Started with `spark run start --project C:\\work\\child`; that folder has no registry entry.
    const childRun = { ...buildRunRecord('run-child', 'C:\\work\\child', 'child.dot'), working_directory: 'C:\\work\\child' }
    const ownRun = buildRunRecord('run-work', 'C:\\work', 'work.dot')
    store.updateRunsListSession({ runs: [childRun, ownRun] })
    store.updateRunDetailSession('run-child', { record: childRun })
    store.updateRunDetailSession('run-work', { record: ownRun })
    store.setRunsSelectedRunId('run-child')

    store.removeProject('C:/work', 'C:/other')

    const next = useStore.getState()
    expect(next.runsListSession.runs.map((run) => run.run_id)).toEqual(['run-child'])
    expect(selectSelectedRunId(next)).toBe('run-child')
    expect(next.runDetailSessionsByRunId['run-child']).toBeDefined()
    expect(next.runDetailSessionsByRunId['run-work']).toBeUndefined()
  })

  it('drops the removed project missions and selection but keeps other projects', () => {
    const store = useStore.getState()
    store.registerProject('/tmp/project-a')
    store.registerProject('/tmp/project-b')
    const mission = (id: string, projectPath: string) => ({ id, project_path: projectPath, revision: 1, fields: { title: id, description: '', archived: false }, activity: [] })
    store.setMissionBoard(() => [mission('m-a', '/tmp/project-a'), mission('m-b', '/tmp/project-b')])
    store.setSelectedMission({ id: 'm-a', projectPath: '/tmp/project-a' })

    store.removeProject('/tmp/project-a', '/tmp/project-b')

    const next = useStore.getState()
    expect(next.missionBoard.map((entry) => entry.id)).toEqual(['m-b'])
    expect(next.selectedMission).toBeNull()
  })

  it('removes conversations, missions and runs recorded with the native Windows path of a removed project', () => {
    const store = useStore.getState()
    store.registerProject('C:\\Users\\chris\\repo')
    store.registerProject('C:\\Users\\chris\\other')
    expect(useStore.getState().projectRegistry['C:/Users/chris/repo']).toBeDefined()
    store.commitHomeConversationCache((cache) => {
      const withRepo = applyConversationWindow(cache, 'conversation-a', 'C:\\Users\\chris\\repo')
      return applyConversationWindow(withRepo, 'conversation-b', 'C:\\Users\\chris\\other')
    })
    store.updateHomeConversationSession('conversation-a', { scrollTop: 120 })
    store.updateHomeConversationSession('conversation-b', { scrollTop: 240 })
    // While registered, each chat is listed under its project's registry key.
    const listed = (projectPath: string) => useStore.getState().homeConversationCache.summariesByProjectPath[projectPath]?.map((summary) => summary.conversation_id)
    expect(listed('C:/Users/chris/repo')).toEqual(['conversation-a'])
    expect(listed('C:/Users/chris/other')).toEqual(['conversation-b'])
    const mission = (id: string, projectPath: string) => ({ id, project_path: projectPath, revision: 1, fields: { title: id, description: '', archived: false }, activity: [] })
    store.setMissionBoard(() => [mission('m-a', 'C:\\Users\\chris\\repo'), mission('m-b', 'C:\\Users\\chris\\other')])
    store.setSelectedMission({ id: 'm-a', projectPath: 'C:\\Users\\chris\\repo' })
    store.updateRunsListSession({ projectFilter: 'C:/Users/chris/repo', runs: [buildRunRecord('run-a', 'C:\\Users\\chris\\repo', 'a.dot')] })

    store.removeProject('C:/Users/chris/repo', 'C:/Users/chris/other')

    const next = useStore.getState()
    expect(next.homeConversationCache.conversationsById['conversation-a']).toBeUndefined()
    expect(next.homeConversationCache.conversationsById['conversation-b']).toBeDefined()
    expect(next.homeConversationCache.summariesByProjectPath['C:/Users/chris/repo']).toBeUndefined()
    expect(listed('C:/Users/chris/other')).toEqual(['conversation-b'])
    expect(next.homeConversationSessionsById['conversation-a']).toBeUndefined()
    expect(next.homeConversationSessionsById['conversation-b']?.scrollTop).toBe(240)
    expect(next.missionBoard.map((entry) => entry.id)).toEqual(['m-b'])
    expect(next.selectedMission).toBeNull()
    expect(next.runsListSession.runs).toEqual([])
    expect(next.runsListSession.projectFilter).toBeNull()
  })

  it('renames normalized home conversation cache entries with project path updates', () => {
    const store = useStore.getState()
    store.registerProject('/tmp/project-a')
    store.commitHomeConversationCache((cache) => applyConversationWindow(cache, 'conversation-a', '/tmp/project-a'))
    store.updateHomeConversationSession('conversation-a', {
      isPinnedToBottom: false,
      scrollTop: 120,
    })

    const result = useStore.getState().updateProjectPath('/tmp/project-a', '/tmp/project-renamed')

    expect(result.ok).toBe(true)
    const next = useStore.getState()
    expect(next.homeConversationCache.conversationsById['conversation-a']?.project_path).toBe('/tmp/project-renamed')
    expect(next.homeConversationCache.summariesByProjectPath['/tmp/project-a']).toBeUndefined()
    expect(next.homeConversationCache.summariesByProjectPath['/tmp/project-renamed']?.[0]).toMatchObject({
      conversation_id: 'conversation-a',
      project_path: '/tmp/project-renamed',
    })
    expect(next.homeConversationSessionsById['conversation-a']?.scrollTop).toBe(120)
  })

  it('removes normalized home conversation cache and scroll sessions with project deletion', () => {
    const store = useStore.getState()
    store.registerProject('/tmp/project-a')
    store.registerProject('/tmp/project-b')
    store.commitHomeConversationCache((cache) => {
      const withProjectA = applyConversationWindow(cache, 'conversation-a', '/tmp/project-a')
      return applyConversationWindow(withProjectA, 'conversation-b', '/tmp/project-b')
    })
    store.updateHomeConversationSession('conversation-a', { scrollTop: 120 })
    store.updateHomeConversationSession('conversation-b', { scrollTop: 240 })

    store.removeProject('/tmp/project-a', '/tmp/project-b')

    const next = useStore.getState()
    expect(next.homeConversationCache.conversationsById['conversation-a']).toBeUndefined()
    expect(next.homeConversationCache.summariesByProjectPath['/tmp/project-a']).toBeUndefined()
    expect(next.homeConversationSessionsById['conversation-a']).toBeUndefined()
    expect(next.homeConversationCache.conversationsById['conversation-b']?.project_path).toBe('/tmp/project-b')
    expect(next.homeConversationSessionsById['conversation-b']?.scrollTop).toBe(240)
  })
})
