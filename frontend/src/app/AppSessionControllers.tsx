import { selectSelectedRunId } from '@/state/runsSessionSelectors'
import { useCallback, useEffect, useMemo, useRef } from 'react'

import { usePersistProjectState } from '@/features/projects/hooks/usePersistProjectState'
import { useConversationStream } from '@/features/projects/hooks/useConversationStream'
import { useProjectConversationCache } from '@/features/projects/hooks/useProjectConversationCache'
import { useProjectGitMetadata } from '@/features/projects/hooks/useProjectGitMetadata'
import { extractApiErrorMessage } from '@/features/projects/model/projectsHomeState'
import { useRunDetailResources } from '@/features/runs/hooks/useRunDetailResources'
import { useRunsList } from '@/features/runs/hooks/useRunsList'
import { useTriggersList } from '@/features/triggers/hooks/useTriggersList'
import { buildWorkspaceLiveEventsUrl } from '@/features/workspace/services/liveEvents'
import type {
    ApplyConversationStreamEventResult,
    ConversationSnapshotResponse,
    ConversationStreamEvent,
    ConversationSummaryResponse,
} from '@/features/projects/model/projectsHomeState'
import type { ConversationStreamDeltaEventResponse } from '@/lib/workspaceClient'
import { useStore } from '@/store'
import { buildRunsScopeKey } from '@/state/runsSessionScope'
import { resolveRunJournalLiveCursor, useRunJournalStore } from '@/features/runs/state/runJournalStore'
import { useRunsTransportReconnectSignal } from '@/features/runs/services/runsTransportReconnect'
import { buildRunsHash, isRunsHash, parseRunsHash } from './runsRouting'
import type { WorkflowLogEntry } from '@/state/workflowEventLogSlice'

type HomeConversationSyncControllerProps = {
    projectPath: string
    conversationId: string | null
    isForegroundProject: boolean
    applyConversationSnapshot: (
        projectPath: string,
        snapshot: ConversationSnapshotResponse,
        source?: string,
    ) => void
    applyConversationStreamEvent: (
        projectPath: string,
        event: ConversationStreamEvent,
        source?: string,
    ) => ApplyConversationStreamEventResult | undefined
    applyTransientConversationEvent: (
        projectPath: string,
        event: ConversationStreamDeltaEventResponse,
        source?: string,
    ) => unknown
    setProjectPanelError: (projectPath: string, value: string | null) => void
}

function HomeConversationSyncController({
    projectPath,
    conversationId,
    isForegroundProject,
    applyConversationSnapshot,
    applyConversationStreamEvent,
    applyTransientConversationEvent,
    setProjectPanelError,
}: HomeConversationSyncControllerProps) {
    const setPanelError = useCallback((value: string | null) => {
        setProjectPanelError(projectPath, value)
    }, [projectPath, setProjectPanelError])

    useConversationStream({
        activeConversationId: isForegroundProject ? conversationId : null,
        activeProjectPath: isForegroundProject ? projectPath : null,
        applyConversationSnapshot,
        applyConversationStreamEvent,
        applyTransientConversationEvent,
        formatErrorMessage: extractApiErrorMessage,
        setPanelError,
    })

    return null
}

type HomeThreadSummariesControllerProps = {
    projectPath: string
    activeConversationId: string | null
    isForegroundProject: boolean
    threadStatus: 'idle' | 'loading' | 'ready' | 'error'
    loadProjectConversationSummaries: (projectPath: string) => Promise<ConversationSummaryResponse[]>
    activateConversationThread: (projectPath: string, conversationId: string) => void
}

function HomeThreadSummariesController({
    projectPath,
    activeConversationId,
    isForegroundProject,
    threadStatus,
    loadProjectConversationSummaries,
    activateConversationThread,
}: HomeThreadSummariesControllerProps) {
    useEffect(() => {
        if (threadStatus !== 'idle') {
            return
        }

        let isCancelled = false

        const loadThreadSummaries = async () => {
            const summaries = await loadProjectConversationSummaries(projectPath)
            if (isCancelled || !isForegroundProject || activeConversationId) {
                return
            }
            const latestConversation = summaries[0] ?? null
            if (latestConversation) {
                activateConversationThread(projectPath, latestConversation.conversation_id)
            }
        }

        void loadThreadSummaries()
        return () => {
            isCancelled = true
        }
    }, [
        activeConversationId,
        activateConversationThread,
        isForegroundProject,
        loadProjectConversationSummaries,
        projectPath,
    ])

    return null
}

export function HomeSessionController() {
    const viewMode = useStore((state) => state.viewMode)
    const projectRegistry = useStore((state) => state.projectRegistry)
    const upsertProjectRegistryEntry = useStore((state) => state.upsertProjectRegistryEntry)
    const activeProjectPath = useStore((state) => state.activeProjectPath)
    const projectSessionsByPath = useStore((state) => state.projectSessionsByPath)
    const homeConversationCache = useStore((state) => state.homeConversationCache)
    const homeThreadSummariesStatusByProjectPath = useStore((state) => state.homeThreadSummariesStatusByProjectPath)
    const homeThreadSummariesErrorByProjectPath = useStore((state) => state.homeThreadSummariesErrorByProjectPath)
    const homeProjectSessionsByPath = useStore((state) => state.homeProjectSessionsByPath)
    const homeProjectGitMetadataByPath = useStore((state) => state.homeProjectGitMetadataByPath)
    const removeHomeProjectSession = useStore((state) => state.removeHomeProjectSession)
    const updateProjectSessionState = useStore((state) => state.updateProjectSessionState)
    const updateHomeProjectSession = useStore((state) => state.updateHomeProjectSession)
    const persistProjectState = usePersistProjectState(upsertProjectRegistryEntry)
    const {
        applyConversationSnapshot,
        applyConversationStreamEvent,
        applyTransientConversationEvent,
        loadProjectConversationSummaries,
    } = useProjectConversationCache({
        persistProjectState,
        projectSessionsByPath,
        updateProjectSessionState,
    })

    const homeProjectPaths = useMemo(() => {
        const nextProjectPaths = new Set<string>()
        const isHomeVisible = viewMode === 'home' || viewMode === 'projects'

        if (isHomeVisible && activeProjectPath) {
            nextProjectPaths.add(activeProjectPath)
        }

        Object.entries(projectSessionsByPath).forEach(([projectPath, session]) => {
            if (session?.conversationId) {
                nextProjectPaths.add(projectPath)
            }
        })
        Object.keys(homeProjectSessionsByPath).forEach((projectPath) => {
            nextProjectPaths.add(projectPath)
        })
        Object.keys(homeThreadSummariesStatusByProjectPath).forEach((projectPath) => {
            nextProjectPaths.add(projectPath)
        })
        Object.keys(homeConversationCache.summariesByProjectPath).forEach((projectPath) => {
            nextProjectPaths.add(projectPath)
        })
        Object.values(homeConversationCache.conversationsById).forEach((conversation) => {
            nextProjectPaths.add(conversation.project_path)
        })

        return [...nextProjectPaths].filter((projectPath) => Boolean(projectRegistry[projectPath]))
    }, [
        activeProjectPath,
        homeConversationCache,
        homeProjectSessionsByPath,
        homeThreadSummariesStatusByProjectPath,
        projectRegistry,
        projectSessionsByPath,
        viewMode,
    ])

    useProjectGitMetadata({
        projectPaths: homeProjectPaths,
        setProjectRegistrationError: () => {},
    })

    const setProjectPanelError = useCallback((projectPath: string, value: string | null) => {
        updateHomeProjectSession(projectPath, { panelError: value })
    }, [updateHomeProjectSession])

    const activateConversationThread = useCallback((projectPath: string, conversationId: string) => {
        updateProjectSessionState(projectPath, { conversationId })
        void persistProjectState(projectPath, {
            active_conversation_id: conversationId,
            last_accessed_at: new Date().toISOString(),
        })
    }, [persistProjectState, updateProjectSessionState])

    useEffect(() => {
        const registeredPaths = new Set(Object.keys(projectRegistry))
        const knownHomeProjectPaths = new Set<string>()

        Object.keys(homeProjectSessionsByPath).forEach((projectPath) => {
            knownHomeProjectPaths.add(projectPath)
        })
        Object.keys(homeThreadSummariesStatusByProjectPath).forEach((projectPath) => {
            knownHomeProjectPaths.add(projectPath)
        })
        Object.keys(homeThreadSummariesErrorByProjectPath).forEach((projectPath) => {
            knownHomeProjectPaths.add(projectPath)
        })
        Object.keys(homeProjectGitMetadataByPath).forEach((projectPath) => {
            knownHomeProjectPaths.add(projectPath)
        })
        Object.keys(homeConversationCache.summariesByProjectPath).forEach((projectPath) => {
            knownHomeProjectPaths.add(projectPath)
        })
        Object.values(homeConversationCache.conversationsById).forEach((conversation) => {
            knownHomeProjectPaths.add(conversation.project_path)
        })

        knownHomeProjectPaths.forEach((projectPath) => {
            if (!registeredPaths.has(projectPath)) {
                removeHomeProjectSession(projectPath)
            }
        })
    }, [
        homeConversationCache,
        homeProjectGitMetadataByPath,
        homeProjectSessionsByPath,
        homeThreadSummariesErrorByProjectPath,
        homeThreadSummariesStatusByProjectPath,
        projectRegistry,
        removeHomeProjectSession,
    ])

    const isHomeVisible = viewMode === 'home' || viewMode === 'projects'

    return (
        <>
            {homeProjectPaths.map((projectPath) => (
                <HomeThreadSummariesController
                    key={`${projectPath}:threads`}
                    projectPath={projectPath}
                    activeConversationId={projectSessionsByPath[projectPath]?.conversationId ?? null}
                    isForegroundProject={isHomeVisible && activeProjectPath === projectPath}
                    threadStatus={homeThreadSummariesStatusByProjectPath[projectPath] ?? 'idle'}
                    loadProjectConversationSummaries={loadProjectConversationSummaries}
                    activateConversationThread={activateConversationThread}
                />
            ))}
            {homeProjectPaths.map((projectPath) => (
                <HomeConversationSyncController
                    key={`${projectPath}:${projectSessionsByPath[projectPath]?.conversationId ?? 'none'}`}
                    projectPath={projectPath}
                    conversationId={projectSessionsByPath[projectPath]?.conversationId ?? null}
                    isForegroundProject={isHomeVisible && activeProjectPath === projectPath}
                    applyConversationSnapshot={applyConversationSnapshot}
                    applyConversationStreamEvent={applyConversationStreamEvent}
                    applyTransientConversationEvent={applyTransientConversationEvent}
                    setProjectPanelError={setProjectPanelError}
                />
            ))}
        </>
    )
}

export function RunsSessionController() {
    const activeProjectPath = useStore((state) => state.activeProjectPath)
    const scopeMode = useStore((state) => state.runsListSession.scopeMode)
    const selectedRunId = useStore(selectSelectedRunId)
    useRunsList({ activeProjectPath, scopeMode, selectedRunId, manageSync: true })
    useRunDetailResources({ selectedRunId, manageSync: true })
    return null
}

export function TriggersSessionController() {
    useTriggersList({ manageSync: true })
    return null
}

export function WorkspaceLiveEventsController() {
    const reconnectSignal = useRunsTransportReconnectSignal(true)
    const viewMode = useStore((state) => state.viewMode)
    const activeProjectPath = useStore((state) => state.activeProjectPath)
    const projectSessionsByPath = useStore((state) => state.projectSessionsByPath)
    const homeConversationCache = useStore((state) => state.homeConversationCache)
    const runsListSession = useStore((state) => state.runsListSession)
    const selectedRunId = useStore(selectSelectedRunId)
    const selectedRunLifetime = useStore((state) => (
        selectedRunId ? state.runDetailSessionsByRunId[selectedRunId]?.lifetime : undefined
    ))
    const selectedRunLiveCursor = useRunJournalStore((state) => (
        selectedRunId ? resolveRunJournalLiveCursor(state.byRunId[selectedRunId]) : null
    ))
    const selectedRunLiveReady = selectedRunLiveCursor !== null
    const activeConversationId = activeProjectPath
        ? projectSessionsByPath[activeProjectPath]?.conversationId ?? null
        : null
    const activeConversationRevision = activeConversationId
        ? homeConversationCache.conversationsById[activeConversationId]?.revision ?? null
        : null
    const latestConversationRevisionById = useRef<Record<string, number>>({})
    const latestRunSequenceById = useRef<Record<string, number>>({})
    const includeRunsOverview =
        viewMode === 'runs'
        || selectedRunId !== null
        || runsListSession.status !== 'idle'
        || runsListSession.runs.length > 0
        || runsListSession.scopeMode !== 'active'
    const includeTriggers = viewMode === 'triggers'
    const conversationProjectPath = activeConversationId ? activeProjectPath : null
    const runsProjectPath = includeRunsOverview && runsListSession.scopeMode === 'active'
        ? activeProjectPath
        : null
    const triggersProjectPath = includeTriggers ? activeProjectPath : null

    useEffect(() => {
        if (
            activeConversationId
            && typeof activeConversationRevision === 'number'
            && Number.isFinite(activeConversationRevision)
        ) {
            latestConversationRevisionById.current[activeConversationId] = activeConversationRevision
        }
    }, [activeConversationId, activeConversationRevision])

    useEffect(() => {
        if (
            selectedRunId
            && typeof selectedRunLiveCursor === 'number'
            && Number.isFinite(selectedRunLiveCursor)
        ) {
            latestRunSequenceById.current[selectedRunId] = selectedRunLiveCursor
        }
    }, [selectedRunId, selectedRunLiveCursor])

    const liveEventsUrl = useMemo(() => {
        const params = new URLSearchParams()
        if (activeConversationId) {
            params.set('conversation_id', activeConversationId)
            if (conversationProjectPath) {
                params.set('conversation_project_path', conversationProjectPath)
            }
        }
        if (selectedRunId && selectedRunLiveReady) {
            params.set('run_id', selectedRunId)
        }
        if (includeRunsOverview) {
            params.set('include_runs_overview', 'true')
            if (runsProjectPath) {
                params.set('runs_project_path', runsProjectPath)
            }
        }
        if (includeTriggers) {
            params.set('include_triggers', 'true')
            if (triggersProjectPath) {
                params.set('triggers_project_path', triggersProjectPath)
            }
        }
        // The workflow event log is a global, always-on feed for the Home pane.
        params.set('include_workflow_log', 'true')
        return buildWorkspaceLiveEventsUrl(params)
    }, [
        activeConversationId,
        conversationProjectPath,
        includeRunsOverview,
        includeTriggers,
        runsProjectPath,
        selectedRunLiveReady,
        selectedRunId,
        triggersProjectPath,
    ])

    useEffect(() => {
        if (typeof EventSource === 'undefined') {
            return
        }
        let eventSource: EventSource | null = null
        let reconnectTimer: ReturnType<typeof window.setTimeout> | null = null
        let closed = false

        const buildUrlWithCursors = () => {
            const url = new URL(liveEventsUrl, window.location.href)
            if (activeConversationId) {
                const revision = latestConversationRevisionById.current[activeConversationId]
                if (typeof revision === 'number' && Number.isFinite(revision)) {
                    url.searchParams.set('conversation_revision', String(revision))
                }
            }
            if (selectedRunId) {
                const sequence = latestRunSequenceById.current[selectedRunId]
                if (typeof sequence === 'number' && Number.isFinite(sequence)) {
                    url.searchParams.set('run_sequence', String(sequence))
                }
            }
            return `${url.pathname}${url.search}`
        }

        const isCurrent = (source: EventSource) => !closed && eventSource === source && (
            !selectedRunId || useStore.getState().runDetailSessionsByRunId[selectedRunId]?.lifetime === selectedRunLifetime
        )

        const handleMessage = (event: MessageEvent<string>, source: EventSource) => {
            if (!isCurrent(source)) return
            try {
                const envelope = JSON.parse(event.data) as {
                    type?: string
                    project_path?: string | null
                    resource?: { kind?: string; id?: string | null }
                    cursor?: { kind?: string; value?: number } | null
                    payload?: unknown
                    sequence?: number
                }
                if (
                    envelope.resource?.kind === 'conversation'
                    && envelope.resource.id
                    && envelope.cursor?.kind === 'conversation_revision'
                    && typeof envelope.cursor.value === 'number'
                    && Number.isFinite(envelope.cursor.value)
                ) {
                    latestConversationRevisionById.current[envelope.resource.id] = envelope.cursor.value
                }
                if (
                    envelope.resource?.kind === 'run'
                    && envelope.resource.id
                    && envelope.cursor?.kind === 'run_sequence'
                    && typeof envelope.cursor.value === 'number'
                    && Number.isFinite(envelope.cursor.value)
                ) {
                    latestRunSequenceById.current[envelope.resource.id] = envelope.cursor.value
                }
                if (envelope.type === 'workflow_log.entry' && envelope.resource?.kind === 'workflow_log') {
                    const entry = envelope.payload as WorkflowLogEntry | undefined
                    if (entry && typeof entry.id === 'string' && typeof entry.seq === 'number') {
                        useStore.getState().applyWorkflowLogEntries([entry])
                    }
                    return
                }
                if (typeof envelope.sequence === 'number' && selectedRunId) {
                    window.dispatchEvent(new CustomEvent('spark:run-journal-entry', {
                        detail: { runId: selectedRunId, entry: envelope },
                    }))
                    return
                }
                const payload = (
                    envelope.payload
                    && typeof envelope.payload === 'object'
                    && !Array.isArray(envelope.payload)
                ) ? envelope.payload as Record<string, unknown> : {}
                if (envelope.type === 'resync_required') {
                    if (envelope.resource?.kind === 'conversation' && envelope.resource.id) {
                        window.dispatchEvent(new CustomEvent('spark:conversation-live-event', {
                            detail: {
                                type: envelope.type,
                                conversationId: envelope.resource.id,
                                projectPath: envelope.project_path ?? activeProjectPath,
                                payload,
                            },
                        }))
                    } else if (envelope.resource?.kind === 'run' && envelope.resource.id) {
                        window.dispatchEvent(new CustomEvent('spark:run-resync-required', {
                            detail: { runId: envelope.resource.id, reason: payload.reason },
                        }))
                    } else if (envelope.resource?.kind === 'runs_overview') {
                        window.dispatchEvent(new CustomEvent('spark:runs-overview-resync-required', {
                            detail: { projectPath: envelope.project_path ?? activeProjectPath, reason: payload.reason },
                        }))
                    } else if (envelope.resource?.kind === 'trigger') {
                        window.dispatchEvent(new CustomEvent('spark:triggers-resync-required', {
                            detail: { projectPath: envelope.project_path ?? activeProjectPath, reason: payload.reason },
                        }))
                    }
                    return
                }
                if (envelope.type === 'run.upsert') {
                    window.dispatchEvent(new CustomEvent('spark:run-upsert', {
                        detail: { run: payload.run },
                    }))
                    return
                }
                if (envelope.type === 'conversation.segment_upsert' && envelope.resource?.kind === 'node_execution') {
                    const record = payload.record && typeof payload.record === 'object' && !Array.isArray(payload.record)
                        ? payload.record as Record<string, unknown> : null
                    for (const runId of new Set([payload.run_id, payload.presentation_run_id])) {
                        if (typeof runId !== 'string' || !runId) continue
                        window.dispatchEvent(new CustomEvent('spark:run-segment-upsert', {
                            detail: {
                                runId,
                                segment: record?.segment && typeof record.segment === 'object'
                                    ? { ...(record.segment as Record<string, unknown>), node_id: payload.node_id,
                                        attempt: payload.attempt, latest_sequence: record.source_event_sequence,
                                        source_scope: runId === payload.run_id ? 'root' : payload.source_scope,
                                        source_run_id: payload.run_id }
                                    : null,
                            },
                        }))
                    }
                    return
                }
                if (
                    (envelope.type === 'run.journal_entry'
                        || envelope.type === 'run.question_pending'
                        || envelope.type === 'run.question_answered')
                    && envelope.resource?.id
                ) {
                    window.dispatchEvent(new CustomEvent('spark:run-journal-entry', {
                        detail: { runId: envelope.resource.id, entry: payload },
                    }))
                    return
                }
                if (envelope.resource?.kind === 'conversation' && envelope.resource.id) {
                    window.dispatchEvent(new CustomEvent('spark:conversation-live-event', {
                        detail: {
                            type: envelope.type,
                            conversationId: envelope.resource.id,
                            projectPath: envelope.project_path ?? activeProjectPath,
                            payload,
                        },
                    }))
                    return
                }
                if (
                    (envelope.type === 'trigger.snapshot'
                        || envelope.type === 'trigger.upsert'
                        || envelope.type === 'trigger.delete')
                    && envelope.resource?.kind === 'trigger'
                ) {
                    window.dispatchEvent(new CustomEvent('spark:trigger-live-event', {
                        detail: {
                            type: envelope.type,
                            projectPath: envelope.project_path ?? activeProjectPath,
                            payload,
                        },
                    }))
                }
            } catch {
                // Ignore malformed stream events.
            }
        }

        const open = () => {
            if (closed) {
                return
            }
            eventSource?.close()
            const source = new EventSource(buildUrlWithCursors())
            eventSource = source
            source.onmessage = (event) => handleMessage(event, source)
            source.onerror = () => {
                if (!isCurrent(source)) {
                    return
                }
                source.close()
                eventSource = null
                if (reconnectTimer !== null) {
                    window.clearTimeout(reconnectTimer)
                }
                reconnectTimer = window.setTimeout(open, 500)
            }
        }

        open()
        return () => {
            closed = true
            if (reconnectTimer !== null) {
                window.clearTimeout(reconnectTimer)
            }
            eventSource?.close()
        }
    }, [activeConversationId, activeProjectPath, liveEventsUrl, reconnectSignal, selectedRunId, selectedRunLifetime])

    return null
}

export function RunsHashRoutingController() {
    const viewMode = useStore((state) => state.viewMode)
    const selectedRunId = useStore(selectSelectedRunId)
    const selectedNodeId = useStore((state) => (
        selectSelectedRunId(state)
            ? state.runDetailSessionsByRunId[selectSelectedRunId(state)!]?.selectedNodeId ?? null
            : null
    ))

    // Hash -> store: applies deep links on load and on hashchange. Store -> hash
    // below uses history.replaceState, which does not fire hashchange, so the
    // two directions cannot loop.
    useEffect(() => {
        const applyHash = () => {
            const route = parseRunsHash(window.location.hash)
            if (!route) {
                return
            }
            const state = useStore.getState()
            if (state.viewMode !== 'runs') {
                state.setViewMode('runs')
            }
            if (selectSelectedRunId(state) !== route.runId) {
                state.setRunsSelectedRunIdForScope(buildRunsScopeKey(state.runsListSession.scopeMode, state.activeProjectPath), route.runId)
            }
            const currentNodeId = state.runDetailSessionsByRunId[route.runId]?.selectedNodeId ?? null
            if (currentNodeId !== route.nodeId) {
                state.updateRunDetailSession(route.runId, {
                    selectedNodeId: route.nodeId,
                    // A node deep link lands on the node inspector tab.
                })
            }
        }
        applyHash()
        window.addEventListener('hashchange', applyHash)
        return () => {
            window.removeEventListener('hashchange', applyHash)
        }
    }, [])

    // Store -> hash: the runs tab owns the hash; other tabs clear a stale runs hash.
    useEffect(() => {
        if (viewMode === 'runs' && selectedRunId) {
            const nextHash = buildRunsHash(selectedRunId, selectedNodeId)
            if (window.location.hash !== nextHash) {
                window.history.replaceState(null, '', nextHash)
            }
        } else if (isRunsHash(window.location.hash)) {
            window.history.replaceState(null, '', window.location.pathname + window.location.search)
        }
    }, [viewMode, selectedRunId, selectedNodeId])

    return null
}

export function AppSessionControllers() {
    return (
        <>
            <WorkspaceLiveEventsController />
            <HomeSessionController />
            <RunsSessionController />
            <TriggersSessionController />
            <RunsHashRoutingController />
        </>
    )
}
