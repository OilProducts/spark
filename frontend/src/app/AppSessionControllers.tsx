import { useLiveThinkingStore } from '@/components/app/transcript/liveThinking'
import { visitKey } from '@/features/runs/model/visitModel'
import { ClientPreferencesController } from '@/features/settings/ClientPreferencesController'
import { ModelDefaultsController } from '@/features/settings/ModelDefaultsController'
import { selectSelectedRunId } from '@/state/runsSessionSelectors'
import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react'
import { AppNotice } from '@/components/app/app-notice'
import { dismissRefusedFlowSave, refusedFlowSaves, refusedFlowSaveStatus, subscribeRefusedFlowSaves } from '@/lib/flowPersistence'

import { usePersistProjectState } from '@/features/projects/hooks/usePersistProjectState'
import { useConversationStream } from '@/features/projects/hooks/useConversationStream'
import { useProjectConversationCache } from '@/features/projects/hooks/useProjectConversationCache'
import { useProjectGitMetadata } from '@/features/projects/hooks/useProjectGitMetadata'
import {
    applySummaryBatchToCache,
    applySummarySnapshotToCache,
    extractApiErrorMessage,
    resetConversationInCache,
    type ConversationSummaryResponse,
} from '@/features/projects/model/projectsHomeState'
import { useRunDetailResources } from '@/features/runs/hooks/useRunDetailResources'
import { useRunsList } from '@/features/runs/hooks/useRunsList'
import { useTriggersList } from '@/features/triggers/hooks/useTriggersList'
import { buildWorkspaceLiveEventsUrl, dispatchConversationLiveEnvelope } from '@/features/workspace/services/liveEvents'
import type { ConversationBatch } from '@/lib/workspaceClient'
import { useStore } from '@/store'
import type { AppState } from '@/state/store-types'
import { openByItself, opensByItself } from '@/state/homeSessionSlice'
import { getActiveEnvironmentId, getActiveEnvironmentState, reverifyActiveEnvironment, subscribeActiveEnvironment } from '@/lib/activeEnvironment'
import { resolveRunJournalLiveCursor, useRunJournalStore } from '@/features/runs/state/runJournalStore'
import { reportRunsTransportLost, requestRunsTransportReconnect, useRunsTransportReconnectSignal } from '@/features/runs/services/runsTransportReconnect'
import { addsEntry, closesTrigger, missionKey, parsePlaceHash, placeHash, placeOf, roleKey, type Place } from './runsRouting'
import { defaultProjectChoice } from '@/features/projects/model/projectChoices'
import { refreshAttention } from './useAttention'

type ConversationStreamArgs = Parameters<typeof useConversationStream>[0]

type HomeConversationSyncControllerProps = Pick<
    ConversationStreamArgs,
    | 'applyConversationPage'
    | 'applyConversationBatch'
    | 'applyTransientConversationEvent'
    | 'resetConversation'
    | 'markConversationUnavailable'
> & {
    projectPath: string
    conversationId: string | null
    isForegroundProject: boolean
    setProjectPanelError: (projectPath: string, value: string | null) => void
}

const getConversationRecord = (conversationId: string) => useStore.getState().homeConversationCache.conversationsById[conversationId]

function HomeConversationSyncController({
    projectPath,
    conversationId,
    isForegroundProject,
    setProjectPanelError,
    ...handlers
}: HomeConversationSyncControllerProps) {
    const setPanelError = useCallback((value: string | null) => {
        setProjectPanelError(projectPath, value)
    }, [projectPath, setProjectPanelError])

    useConversationStream({
        activeConversationId: isForegroundProject ? conversationId : null,
        activeProjectPath: isForegroundProject ? projectPath : null,
        getConversationRecord,
        ...handlers,
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
                openByItself(() => activateConversationThread(projectPath, latestConversation.conversation_id))
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
        applyConversationBatch,
        applyConversationPage,
        applyTransientConversationEvent,
        loadProjectConversationSummaries,
        markConversationUnavailable,
        resetConversation,
    } = useProjectConversationCache({
        persistProjectState,
        projectSessionsByPath,
        updateProjectSessionState,
    })

    // The Chats panel lists every project's chats.
    const homeProjectPaths = useMemo(() => Object.keys(projectRegistry), [projectRegistry])

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
        // A loaded registry always holds Home; until it loads, every project would look removed,
        // and pruning then drops a draft typed into the restored chat.
        if (registeredPaths.size === 0) {
            return
        }
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
                    applyConversationPage={applyConversationPage}
                    applyConversationBatch={applyConversationBatch}
                    applyTransientConversationEvent={applyTransientConversationEvent}
                    resetConversation={resetConversation}
                    markConversationUnavailable={markConversationUnavailable}
                    setProjectPanelError={setProjectPanelError}
                />
            ))}
        </>
    )
}

export function RunsSessionController() {
    const selectedRunId = useStore(selectSelectedRunId)
    useRunsList({ selectedRunId, manageSync: true })
    useRunDetailResources({ selectedRunId, manageSync: true })
    return null
}

export function TriggersSessionController() {
    useTriggersList({ manageSync: true })
    return null
}

export function WorkspaceLiveEventsController() {
    const reconnectSignal = useRunsTransportReconnectSignal(true)
    // The live connection failed since one last opened and passed its check; it outlasts restarts, such as Try again's.
    const unverified = useRef(false)
    const activeProjectPath = useStore((state) => state.activeProjectPath)
    const projectSessionsByPath = useStore((state) => state.projectSessionsByPath)
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
    const latestRunSequenceById = useRef<Record<string, number>>({})
    const conversationProjectPath = activeConversationId ? activeProjectPath : null

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
        // Runs, triggers and missions follow every project, so every list stays live.
        params.set('include_runs_overview', 'true')
        params.set('include_triggers', 'true')
        params.set('include_missions', 'true')
        params.set('include_conversations', 'true')
        params.set('include_settings', 'true')
        return buildWorkspaceLiveEventsUrl(params)
    }, [
        activeConversationId,
        conversationProjectPath,
        selectedRunLiveReady,
        selectedRunId,
    ])

    useEffect(() => {
        if (typeof EventSource === 'undefined') {
            return
        }
        let eventSource: EventSource | null = null
        let reconnectTimer: ReturnType<typeof window.setTimeout> | null = null
        let closed = false
        // Failed connects in a row: two or more mean Spark was unreachable, not a routine drop.
        let failedConnects = 0
        // What a reconnected linked environment sends before its check passes, which applies it.
        let held: MessageEvent<string>[] | null = null
        let stopWaiting = () => {}
        // The environment this connection belongs to; another one mounted remounts the workspace with its own.
        const environmentId = getActiveEnvironmentId()

        // Summary changes that arrive before a cursorless connection's snapshot wait for it.
        let awaitingSummarySnapshot = false
        let bufferedSummaryChanges: ConversationBatch[] = []

        const buildUrlWithCursors = () => {
            const url = new URL(liveEventsUrl, window.location.href)
            const cache = useStore.getState().homeConversationCache
            // Each scope resumes from its own watermark; without one the server sends its initial state.
            const record = activeConversationId ? cache.conversationsById[activeConversationId] : undefined
            if (record && record.watermark !== null && !record.resetting) {
                url.searchParams.set('conversation_generation', record.generation)
                url.searchParams.set('conversation_revision', record.watermark)
            }
            if (cache.summaryCursor) {
                url.searchParams.set('conversations_generation', cache.summaryCursor.generation)
                url.searchParams.set('conversations_revision', cache.summaryCursor.watermark)
            }
            awaitingSummarySnapshot = !cache.summaryCursor
            bufferedSummaryChanges = []
            if (selectedRunId) {
                const sequence = latestRunSequenceById.current[selectedRunId]
                if (typeof sequence === 'number' && Number.isFinite(sequence)) {
                    url.searchParams.set('run_sequence', String(sequence))
                }
            }
            return `${url.pathname}${url.search}`
        }

        const commitCache = useStore.getState().commitHomeConversationCache
        const conversationHandlers = {
            onSummarySnapshot: (snapshot: Parameters<typeof applySummarySnapshotToCache>[1]) => {
                const buffered = bufferedSummaryChanges
                awaitingSummarySnapshot = false
                bufferedSummaryChanges = []
                commitCache((current) => buffered.reduce(
                    applySummaryBatchToCache,
                    applySummarySnapshotToCache(current, snapshot, 'live'),
                ))
            },
            onSummaryChanges: (batch: ConversationBatch) => {
                if (awaitingSummarySnapshot) {
                    bufferedSummaryChanges.push(batch)
                    return
                }
                commitCache((current) => applySummaryBatchToCache(current, batch))
            },
            onSummaryReset: () => {
                commitCache((current) => ({ ...current, summaryCursor: null }))
                open()
            },
            onConversationReset: (conversationId: string) => {
                commitCache((current) => resetConversationInCache(current, conversationId))
                open()
            },
        }

        const isCurrent = (source: EventSource) => !closed && eventSource === source && (
            !selectedRunId || useStore.getState().runDetailSessionsByRunId[selectedRunId]?.lifetime === selectedRunLifetime
        )

        const handleMessage = (event: MessageEvent<string>, source: EventSource) => {
            if (!isCurrent(source)) return
            if (held) {
                held.push(event)
                return
            }
            failedConnects = 0
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
                    envelope.resource?.kind === 'run'
                    && envelope.resource.id
                    && envelope.cursor?.kind === 'run_sequence'
                    && typeof envelope.cursor.value === 'number'
                    && Number.isFinite(envelope.cursor.value)
                ) {
                    // The server subscribes before reading initial state, so an
                    // entry committed in between arrives twice; drop the repeat.
                    const previous = latestRunSequenceById.current[envelope.resource.id]
                    if (typeof previous === 'number' && envelope.cursor.value <= previous) return
                    latestRunSequenceById.current[envelope.resource.id] = envelope.cursor.value
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
                if (envelope.type === 'harness.checked') {
                    window.dispatchEvent(new Event('spark:harness-checked'))
                    return
                }
                if (envelope.type === 'settings.changed' || (envelope.type === 'resync_required' && envelope.resource?.kind === 'settings')) {
                    window.dispatchEvent(new CustomEvent('spark:settings-live-event', { detail: envelope }))
                    return
                }
                if (envelope.type === 'resync_required') {
                    if (envelope.resource?.kind === 'run' && envelope.resource.id) {
                        window.dispatchEvent(new CustomEvent('spark:run-resync-required', {
                            detail: { runId: envelope.resource.id, reason: payload.reason },
                        }))
                    } else if (envelope.resource?.kind === 'runs_overview') {
                        window.dispatchEvent(new CustomEvent('spark:runs-overview-resync-required', {
                            detail: { projectPath: envelope.project_path ?? null, reason: payload.reason },
                        }))
                    } else if (envelope.resource?.kind === 'mission') {
                        window.dispatchEvent(new CustomEvent('spark:mission-live-event', {
                            detail: { projectPath: envelope.project_path ?? null, mission: null },
                        }))
                    } else if (envelope.resource?.kind === 'trigger') {
                        window.dispatchEvent(new CustomEvent('spark:triggers-resync-required', {
                            detail: { projectPath: envelope.project_path ?? null, reason: payload.reason },
                        }))
                    }
                    return
                }
                if (dispatchConversationLiveEnvelope(envelope, conversationHandlers)) {
                    return
                }
                if (envelope.type === 'mission.upsert' && envelope.resource?.kind === 'mission') {
                    window.dispatchEvent(new CustomEvent('spark:mission-live-event', {
                        detail: { projectPath: envelope.project_path ?? null, mission: payload.mission },
                    }))
                    return
                }
                if (envelope.type === 'run.upsert') {
                    window.dispatchEvent(new CustomEvent('spark:run-upsert', {
                        detail: { run: payload.run },
                    }))
                    return
                }
                if (envelope.type === 'run.thinking') {
                    if (
                        typeof payload.run_id === 'string' && typeof payload.node_id === 'string'
                        && typeof payload.stage_index === 'number' && typeof payload.attempt === 'number'
                    ) {
                        useLiveThinkingStore.getState().report(
                            visitKey(payload.run_id, payload.node_id, payload.stage_index, payload.attempt),
                            payload.active === true,
                            typeof payload.estimated_tokens === 'number' ? payload.estimated_tokens : null,
                            null,
                            typeof payload.elapsed_ms === 'number' ? payload.elapsed_ms : null,
                        )
                    }
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
                                        stage_index: payload.stage_index, attempt: payload.attempt,
                                        latest_sequence: record.source_event_sequence,
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
                if (
                    (envelope.type === 'trigger.snapshot'
                        || envelope.type === 'trigger.upsert'
                        || envelope.type === 'trigger.delete')
                    && envelope.resource?.kind === 'trigger'
                ) {
                    window.dispatchEvent(new CustomEvent('spark:trigger-live-event', {
                        detail: {
                            type: envelope.type,
                            projectPath: envelope.project_path ?? null,
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
            held = null
            const source = new EventSource(buildUrlWithCursors())
            eventSource = source
            const reconnect = () => {
                if (reconnectTimer !== null) {
                    window.clearTimeout(reconnectTimer)
                }
                reconnectTimer = window.setTimeout(open, 500)
            }
            const connected = () => {
                // This connection's first frames restore whatever is still thinking; the rest ended unseen.
                useLiveThinkingStore.getState().clear()
                window.dispatchEvent(new Event('spark:settings-live-event'))
                // Back after an outage: reload what failed meanwhile, as Reconnect would.
                if (failedConnects >= 2) {
                    refreshAttention()
                    requestRunsTransportReconnect()
                }
                failedConnects = 0
                unverified.current = false
            }
            // The connection stays closed, its workspace behind the gate, until a check passes, such as Retry's.
            const waitForCheck = () => {
                if (reconnectTimer !== null) window.clearTimeout(reconnectTimer)
                eventSource?.close()
                eventSource = null
                stopWaiting()
                stopWaiting = subscribeActiveEnvironment(() => {
                    const { activeEnvironmentId, selectedEnvironmentId, status } = getActiveEnvironmentState()
                    if (activeEnvironmentId !== environmentId) return stopWaiting()
                    if (selectedEnvironmentId !== activeEnvironmentId || status !== 'ready') return
                    stopWaiting()
                    open()
                })
            }
            source.onopen = () => {
                if (!isCurrent(source)) return
                if (!unverified.current || getActiveEnvironmentId() === null) return connected()
                // Back after an error, a linked environment may since have been upgraded, re-paired or have revoked this
                // Spark's credential: what this connection sends, and the reloads it starts, wait for a check made now.
                held = []
                void reverifyActiveEnvironment(true).then((verified) => {
                    if (!isCurrent(source)) return
                    const frames = held ?? []
                    held = null
                    if (verified) {
                        connected()
                        frames.forEach((frame) => handleMessage(frame, source))
                        return
                    }
                    source.close()
                    eventSource = null
                    if (verified === null) return reconnect()
                    waitForCheck()
                })
            }
            source.onmessage = (event) => handleMessage(event, source)
            source.onerror = () => {
                if (!isCurrent(source)) {
                    return
                }
                source.close()
                eventSource = null
                failedConnects += 1
                unverified.current = true
                // A linked environment found incompatible, or refusing this link until it is paired again, stays so
                // however often it is tried, even when its handshake never opens: none is tried until a check passes.
                // One down meanwhile is retried.
                // ponytail: a check per failed attempt while a peer is down; back off if that load matters. The proxy's
                // 502 for a peer answering as another environment reads like a gateway outage, so it is retried as one.
                void reverifyActiveEnvironment().then(() => {
                    const { status } = getActiveEnvironmentState().activeCheck
                    if (!closed && (status === 'incompatible' || status === 'refused')) waitForCheck()
                })
                // Spark is unreachable: runs say so instead of looking live.
                if (failedConnects === 2) reportRunsTransportLost()
                reconnect()
            }
        }

        open()
        return () => {
            closed = true
            stopWaiting()
            if (reconnectTimer !== null) {
                window.clearTimeout(reconnectTimer)
            }
            eventSource?.close()
        }
    }, [activeConversationId, liveEventsUrl, reconnectSignal, selectedRunId, selectedRunLifetime])

    return null
}

/** The project a link without one opens its item in: the one the app shows. */
function projectInView() {
    const state = useStore.getState()
    return state.selectedRole?.projectPath ?? state.activeProjectPath ?? defaultProjectChoice(state.projectRegistry)
}

/**
 * Whether a history entry's or a link's place is in another environment than the one shown: another
 * environment's selections never move this one, nor its saved route.
 */
const isForeignPlace = (place: Place) => (place.envId ?? null) !== getActiveEnvironmentId()

/**
 * What you choose here that no place names: the project open in Chats, the settings category, a project's
 * Settings, the New Trigger form. Its chat is `openChat`.
 */
function chosenHere() {
    const state = useStore.getState()
    return [state.activeProjectPath, state.settingsCategory, state.projectPageSettingsRequest, state.triggersSession.createFormOpen]
}

const openChat = (state: AppState) => state.activeProjectPath ? state.projectSessionsByPath[state.activeProjectPath]?.conversationId ?? null : null

/** Moves the app to a place from a history entry or a link; returns whether it got there now. */
function applyPlace(place: Place): boolean {
    const state = useStore.getState()
    if (place.runId !== undefined && selectSelectedRunId(state) !== place.runId) {
        state.setRunsSelectedRunId(place.runId)
    }
    if (place.runId && place.nodeId !== undefined
        && useStore.getState().runDetailSessionsByRunId[place.runId]?.selectedNodeId !== place.nodeId) {
        state.updateRunDetailSession(place.runId, { selectedNodeId: place.nodeId })
    }
    if (place.projectPagePath !== undefined && state.projectPagePath !== place.projectPagePath) {
        state.openProjectPage(place.projectPagePath)
    }
    if (place.projectPath !== undefined && state.activeProjectPath !== place.projectPath) {
        state.setActiveProjectPath(place.projectPath)
    }
    // A flow's name only names it within its project, so the flow opens once its project has.
    const { activeProjectPath, activeFlow } = useStore.getState()
    if (place.flow !== undefined && activeFlow !== place.flow
        && (place.projectPath === undefined || activeProjectPath === place.projectPath)) {
        state.setActiveFlow(place.flow)
    }
    if (place.triggerId !== undefined && state.triggersSession.selectedTriggerId !== place.triggerId) {
        state.updateTriggersSession({ selectedTriggerId: place.triggerId, createFormOpen: false })
    }
    if (place.missionId !== undefined && missionKey(state.selectedMission) !== place.missionId) {
        const [projectPath, id] = place.missionId ? JSON.parse(place.missionId) as [string, string] : [null, null]
        state.setSelectedMission(projectPath && id ? { projectPath, id } : null)
    }
    if (place.roleId !== undefined && roleKey(state.selectedRole) !== place.roleId) {
        const [projectPath, name] = place.roleId ? JSON.parse(place.roleId) as [string, string] : [null, null]
        state.setSelectedRole(projectPath && name ? { projectPath, name } : null)
    }
    if (placeOf(useStore.getState()).view !== place.view) {
        state.setViewMode(place.view)
    }
    // Unsaved settings or project edits can hold the move behind a confirmation.
    const reached = placeOf(useStore.getState())
    return (Object.keys(place) as (keyof Place)[])
        .every((field) => place[field] === undefined || reached[field] === place[field])
}

// The environment routing last ran in. The workspace unmounts while another environment connects; shown
// again in the same one, as after that connection fails, it keeps the entry shown and those after it.
let routedEnvironment: string | null | undefined

/**
 * Keeps the browser's Back and Forward in step with the app: opening a view, a run, a project
 * page, a flow, a trigger, a mission or a role adds a history entry, and the URL hash names it so a link reopens it.
 */
export function HistoryRoutingController() {
    useEffect(() => {
        // This routes the environment it mounted in. A switch resets the store before it unmounts,
        // and from then on it records and applies nothing, or the reset would add an entry.
        const environmentId = getActiveEnvironmentId()
        const retired = () => getActiveEnvironmentId() !== environmentId
        const hash = window.location.hash
        // The entry's place, which keeps what its hash can't name, such as a project page; else the hash's.
        const entry = window.history.state?.sparkPlace as Place | undefined
        const linked = entry ?? parsePlaceHash(hash, projectInView())
        // Another environment's link (a reload while one was shown starts on this machine) opens nothing.
        const foreign = linked !== null && isForeignPlace(linked)
        if (linked && !foreign) applyPlace(linked)
        let current = placeOf(useStore.getState())
        let chosen = chosenHere()
        let projects = useStore.getState().projectRegistry
        // Another environment's entry, or its link followed, which has no entry of its own, stays for Back and Forward
        // to reach: a switch here adds this one's after it, and shown again here, as after a failed connection, it stays.
        // A page opened on such a link replaces it, so one Back returns to where it was followed from.
        const pageOpened = routedEnvironment === undefined && !entry
        const switched = routedEnvironment !== environmentId
        routedEnvironment = environmentId
        const shown = parsePlaceHash(hash)
        if (pageOpened || (!(entry && isForeignPlace(entry)) && !(shown && isForeignPlace(shown)))) {
            window.history.replaceState({ sparkPlace: current }, '', placeHash(current))
        } else if (switched) window.history.pushState({ sparkPlace: current }, '', placeHash(current))
        // A trigger, mission or role link without a project names its item in one once the projects load,
        // unless you have moved on by then.
        const opened = placeHash(current)
        const stopWaiting: () => void = projectInView() || foreign ? () => {} : useStore.subscribe(() => {
            const project = projectInView()
            if (!project || retired()) return
            stopWaiting()
            const place = parsePlaceHash(hash, project)
            if (place && placeHash(placeOf(useStore.getState())) === opened) applyPlace(place)
        })

        // One click can change the view and its selection in separate updates; they make one entry.
        let scheduled = false
        let chatChosen = false
        const record = () => {
            scheduled = false
            if (retired()) return
            const state = useStore.getState()
            const next = placeOf(state)
            const entry = window.history.state?.sparkPlace as Place | undefined
            const shown = parsePlaceHash(window.location.hash)
            if ((entry && isForeignPlace(entry)) || (shown && isForeignPlace(shown))) {
                // Back or Forward left another environment's entry showing: a move here follows it, never
                // rewriting it. What moves by itself leaves history as it is: the projects loading, which
                // can open a project and its chat, a project's saved state answering, and what `addsEntry`
                // finds opened by itself, such as a run. The projects have loaded once they hold Home, as a
                // loaded list always does, so the first project added before then or without it is yours.
                const loading = !Object.values(projects).some((project) => project.isDefault)
                    && Object.values(state.projectRegistry).some((project) => project.isDefault)
                const moved = addsEntry(state, current, next)
                    // Only you choose a node of the open run, close a trigger the list still has, and what no place names.
                    || (next.runId === current.runId && next.nodeId !== current.nodeId)
                    || closesTrigger(state, current, next)
                    || chosenHere().some((value, index) => value !== chosen[index])
                    || chatChosen
                    // A project added or removed; an answer saving one's state only updates it.
                    || Object.keys(state.projectRegistry).join('\n') !== Object.keys(projects).join('\n')
                if (moved && !loading) window.history.pushState({ sparkPlace: next }, '', placeHash(next))
            } else if (addsEntry(state, current, next)) {
                window.history.pushState({ sparkPlace: next }, '', placeHash(next))
            } else if (placeHash(next) !== window.location.hash) {
                window.history.replaceState({ sparkPlace: next }, '', placeHash(next))
            }
            current = next
            chosen = chosenHere()
            projects = state.projectRegistry
            chatChosen = false
        }
        // The store as this last saw it. A change made inside another listener, such as the app opening a chat by
        // itself, reaches later listeners again afterwards with the state before the change that started it as
        // `previous`; against this, it counts once, as it was made.
        let seen = useStore.getState()
        const unsubscribe = useStore.subscribe((state) => {
            // A chat you choose opens alone; one the projects loading or a project's saved state answering
            // opens comes with the projects, and the app's own choice of a project's latest is no one's.
            if (state.projectRegistry === seen.projectRegistry && openChat(state) !== openChat(seen) && !opensByItself()) chatChosen = true
            seen = state
            if (scheduled) return
            scheduled = true
            queueMicrotask(record)
        })

        // Back, Forward, or a followed link: the entry's place, or else the one its hash names.
        // A link that names no place, or one it can't reach now, leaves the app where it is, and the
        // link's entry then says where. Another environment's entry is left as it is, app and entry
        // both, so Back and Forward still reach it once that environment shows again.
        const onNavigate = () => {
            if (retired()) return
            const entry = window.history.state?.sparkPlace as Place | undefined
            const place = entry ?? parsePlaceHash(window.location.hash, projectInView())
            if (place !== null && isForeignPlace(place)) return
            const held = place !== null && !applyPlace(place)
            current = placeOf(useStore.getState())
            // Back or Forward held is still where it was: put that entry back, so the URL and Back stay true.
            window.history[held && entry ? 'pushState' : 'replaceState']({ sparkPlace: current }, '', placeHash(current))
        }
        window.addEventListener('popstate', onNavigate)
        window.addEventListener('hashchange', onNavigate)
        return () => {
            unsubscribe()
            stopWaiting()
            window.removeEventListener('popstate', onNavigate)
            window.removeEventListener('hashchange', onNavigate)
        }
    }, [])

    return null
}

/**
 * Flow saves this environment refused out of sight, by flow and project, until dismissed. Behind another
 * view of the editor that still has the edits, the notice reports them; the editor shows them itself.
 * Otherwise it shows them to review and make again by hand; nothing sends them over the flow.
 */
export function RefusedFlowSaveNotices() {
    const [environmentId] = useState(getActiveEnvironmentId)
    const refused = useSyncExternalStore(subscribeRefusedFlowSaves, refusedFlowSaves)
    const editorShown = useStore((state) => state.viewMode === 'editor')
    return refused.filter((save) => save.environmentId === environmentId).flatMap((save) => {
        const status = refusedFlowSaveStatus(save)
        if (status === 'open' && editorShown) return []
        const { content } = save
        const edits = 'flow' in content ? JSON.stringify(content.flow, null, 2)
            : [...content.files.map((file) => `${file.name}:\n${file.content}`), ...(content.deleted_files ?? []).map((file) => `${file}: deleted`)].join('\n')
        const next = status === 'open' ? 'Its edits are still in the editor.'
            : 'These edits weren’t saved. Review and copy them below if you still want to apply them.'
        return [
            <AppNotice
                key={`${save.projectPath}\n${save.name}`}
                data-testid="refused-flow-save"
                // The same flow name in another project is another flow, with its own edits.
                title={`Couldn’t save ${save.name}${save.projectPath ? ` in ${save.projectPath}` : ''}`}
                onDismiss={() => dismissRefusedFlowSave(save)}
                actions={status === 'open' ? undefined : (
                    <details className="w-full">
                        <summary>These edits</summary>
                        <pre data-testid="refused-flow-save-edits" className="max-h-48 overflow-auto whitespace-pre-wrap">{edits}</pre>
                    </details>
                )}
            >
                {`${save.message} ${next}`}
            </AppNotice>,
        ]
    })
}

/**
 * Problems with the whole app, as banners above the open view. They take their own room rather
 * than float, so they never cover a control such as a composer's Send button.
 */
export function AppNotices() {
    return (
        <div data-testid="app-notices" className="flex shrink-0 flex-col gap-2 px-3 pt-3 empty:hidden">
            <ModelDefaultsController />
            <ClientPreferencesController />
            <RefusedFlowSaveNotices />
        </div>
    )
}

export function AppSessionControllers() {
    return (
        <>
            <WorkspaceLiveEventsController />
            <HomeSessionController />
            <RunsSessionController />
            <TriggersSessionController />
            <HistoryRoutingController />
        </>
    )
}
