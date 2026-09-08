import { useCallback, useEffect } from 'react'

import {
    ApiHttpError,
    fetchPipelineArtifactPreviewValidated,
    fetchPipelineArtifactsValidated,
    fetchPipelineCheckpointValidated,
    fetchPipelineContextValidated,
    fetchPipelineQuestionsValidated,
    fetchPipelineResultValidated,
    pipelineArtifactHref,
} from '@/lib/attractorClient'
import { useStore } from '@/store'

import type {
    ArtifactErrorState,
    ArtifactListResponse,
    CheckpointErrorState,
    CheckpointResponse,
    ContextErrorState,
    ContextResponse,
    PendingQuestionSnapshot,
} from '../model/shared'
import {
    artifactErrorFromResponse,
    artifactPreviewErrorFromResponse,
    asPendingQuestionSnapshot,
    checkpointErrorFromResponse,
    contextErrorFromResponse,
    logUnexpectedRunError,
} from '../model/runDetailsModel'

type UseRunDetailResourcesArgs = {
    selectedRunId: string | null
    manageSync?: boolean
}

let nextArtifactPreviewRequestId = 0
let nextResourceRequestId = 0

function beginResourceRequest(runId: string, resource: string) {
    const state = useStore.getState()
    const session = state.runDetailSessionsByRunId[runId]
    if (!session) return null
    const requestId = ++nextResourceRequestId
    state.updateRunDetailSession(runId, { resourceRequestIds: { ...session.resourceRequestIds, [resource]: requestId } })
    return () => useStore.getState().runDetailSessionsByRunId[runId]?.resourceRequestIds[resource] === requestId
}

const DEFAULT_RUN_DETAIL_SESSION = {
    checkpointData: null as CheckpointResponse | null,
    checkpointStatus: 'idle' as const,
    checkpointError: null as CheckpointErrorState | null,
    contextData: null as ContextResponse | null,
    contextStatus: 'idle' as const,
    contextError: null as ContextErrorState | null,
    contextSearchQuery: '',
    contextCopyStatus: '',
    resultData: null as import('@/lib/attractorClient').PipelineResultResponse | null,
    resultStatus: 'idle' as const,
    resultError: null as string | null,
    artifactData: null as ArtifactListResponse | null,
    artifactStatus: 'idle' as const,
    artifactError: null as ArtifactErrorState | null,
    selectedArtifactPath: null as string | null,
    artifactViewerStatus: 'idle' as const,
    artifactViewerPayload: '',
    artifactViewerError: null as string | null,
    questionsStatus: 'idle' as const,
    pendingQuestionSnapshots: [] as PendingQuestionSnapshot[],
}

export function useRunDetailResources({
    selectedRunId,
    manageSync = true,
}: UseRunDetailResourcesArgs) {
    const session = useStore((state) => selectedRunId ? state.runDetailSessionsByRunId[selectedRunId] ?? DEFAULT_RUN_DETAIL_SESSION : DEFAULT_RUN_DETAIL_SESSION)
    const updateRunDetailSession = useStore((state) => state.updateRunDetailSession)

    const fetchCheckpoint = useCallback(async () => {
        if (!selectedRunId) {
            return
        }
        const isCurrentRequest = beginResourceRequest(selectedRunId, 'checkpoint')
        if (!isCurrentRequest) return
        updateRunDetailSession(selectedRunId, {
            checkpointStatus: 'loading',
            checkpointError: null,
        })
        try {
            const payload = await fetchPipelineCheckpointValidated(selectedRunId) as CheckpointResponse
            if (!isCurrentRequest()) return
            updateRunDetailSession(selectedRunId, {
                checkpointData: payload,
                checkpointStatus: 'ready',
                checkpointError: null,
            })
        } catch (err) {
            if (!isCurrentRequest()) return
            logUnexpectedRunError(err)
            updateRunDetailSession(selectedRunId, {
                checkpointStatus: 'error',
                checkpointError: err instanceof ApiHttpError
                    ? checkpointErrorFromResponse(err.status, err.detail)
                    : {
                        message: 'Unable to load checkpoint.',
                        help: 'Check your network/backend connection and retry.',
                    },
            })
        }
    }, [selectedRunId, updateRunDetailSession])

    const fetchContext = useCallback(async () => {
        if (!selectedRunId) {
            return
        }
        const isCurrentRequest = beginResourceRequest(selectedRunId, 'context')
        if (!isCurrentRequest) return
        updateRunDetailSession(selectedRunId, {
            contextStatus: 'loading',
            contextError: null,
        })
        try {
            const payload = await fetchPipelineContextValidated(selectedRunId) as ContextResponse
            if (!isCurrentRequest()) return
            updateRunDetailSession(selectedRunId, {
                contextData: payload,
                contextStatus: 'ready',
                contextError: null,
            })
        } catch (err) {
            if (!isCurrentRequest()) return
            logUnexpectedRunError(err)
            updateRunDetailSession(selectedRunId, {
                contextStatus: 'error',
                contextError: err instanceof ApiHttpError
                    ? contextErrorFromResponse(err.status, err.detail)
                    : {
                        message: 'Unable to load context.',
                        help: 'Check your network/backend connection and retry.',
                    },
            })
        }
    }, [selectedRunId, updateRunDetailSession])

    const fetchArtifacts = useCallback(async () => {
        if (!selectedRunId) {
            return
        }
        const isCurrentRequest = beginResourceRequest(selectedRunId, 'artifact')
        if (!isCurrentRequest) return
        updateRunDetailSession(selectedRunId, {
            artifactStatus: 'loading',
            artifactError: null,
        })
        try {
            const payload = await fetchPipelineArtifactsValidated(selectedRunId)
            if (!isCurrentRequest()) return
            updateRunDetailSession(selectedRunId, {
                artifactData: payload,
                artifactStatus: 'ready',
                artifactError: null,
            })
        } catch (err) {
            if (!isCurrentRequest()) return
            logUnexpectedRunError(err)
            updateRunDetailSession(selectedRunId, {
                artifactStatus: 'error',
                artifactError: err instanceof ApiHttpError
                    ? artifactErrorFromResponse(err.status, err.detail)
                    : {
                        message: 'Unable to load artifacts.',
                        help: 'Check your network/backend connection and retry.',
                    },
            })
        }
    }, [selectedRunId, updateRunDetailSession])

    const fetchResult = useCallback(async () => {
        if (!selectedRunId) {
            return
        }
        const isCurrentRequest = beginResourceRequest(selectedRunId, 'result')
        if (!isCurrentRequest) return
        updateRunDetailSession(selectedRunId, {
            resultStatus: 'loading',
            resultError: null,
        })
        try {
            const payload = await fetchPipelineResultValidated(selectedRunId)
            if (!isCurrentRequest()) return
            updateRunDetailSession(selectedRunId, {
                resultData: payload,
                resultStatus: 'ready',
                resultError: null,
            })
        } catch (err) {
            if (!isCurrentRequest()) return
            logUnexpectedRunError(err)
            updateRunDetailSession(selectedRunId, {
                resultStatus: 'error',
                resultError: err instanceof ApiHttpError
                    ? String(err.detail || 'Unable to load result.')
                    : 'Unable to load result. Check your network/backend connection and retry.',
            })
        }
    }, [selectedRunId, updateRunDetailSession])

    const fetchPendingQuestions = useCallback(async () => {
        if (!selectedRunId) {
            return
        }
        const isCurrentRequest = beginResourceRequest(selectedRunId, 'questions')
        if (!isCurrentRequest) return
        updateRunDetailSession(selectedRunId, {
            questionsStatus: 'loading',
        })
        try {
            const payload = await fetchPipelineQuestionsValidated(selectedRunId)
            if (!isCurrentRequest()) return
            const rawQuestions = payload.questions
            const parsedQuestions = Array.isArray(rawQuestions)
                ? rawQuestions
                    .map((question) => asPendingQuestionSnapshot(question))
                    .filter((question): question is PendingQuestionSnapshot => question !== null)
                : []
            updateRunDetailSession(selectedRunId, {
                pendingQuestionSnapshots: parsedQuestions,
                questionsStatus: 'ready',
            })
        } catch (error) {
            if (!isCurrentRequest()) return
            logUnexpectedRunError(error)
            updateRunDetailSession(selectedRunId, {
                questionsStatus: 'error',
            })
        }
    }, [selectedRunId, updateRunDetailSession])

    useEffect(() => {
        if (!manageSync || !selectedRunId) {
            return
        }
        void fetchCheckpoint()
        void fetchContext()
        void fetchResult()
        void fetchArtifacts()
        void fetchPendingQuestions()
        const refreshQuestions = (event: Event) => {
            const detail = event instanceof CustomEvent ? event.detail : null
            const type = detail?.entry?.raw_type ?? detail?.entry?.type
            if (detail?.runId === selectedRunId && (type === 'human_gate' || type === 'InterviewStarted')) void fetchPendingQuestions()
        }
        window.addEventListener('spark:run-journal-entry', refreshQuestions)
        return () => window.removeEventListener('spark:run-journal-entry', refreshQuestions)
    }, [fetchArtifacts, fetchCheckpoint, fetchContext, fetchPendingQuestions, fetchResult, manageSync, selectedRunId])

    const viewArtifact = useCallback(async (entry: { path: string; viewable: boolean }) => {
        if (!selectedRunId) {
            return
        }
        const requestId = ++nextArtifactPreviewRequestId
        const isCurrentRequest = () => (
            useStore.getState().runDetailSessionsByRunId[selectedRunId]?.artifactViewerRequestId === requestId
        )
        updateRunDetailSession(selectedRunId, {
            selectedArtifactPath: entry.path,
            artifactViewerRequestId: requestId,
            artifactViewerPayload: '',
            artifactViewerError: null,
        })
        if (!entry.viewable) {
            updateRunDetailSession(selectedRunId, {
                artifactViewerStatus: 'error',
                artifactViewerError: 'Preview unavailable for this artifact type. Use download action.',
            })
            return
        }
        updateRunDetailSession(selectedRunId, {
            artifactViewerStatus: 'loading',
        })
        try {
            const payload = await fetchPipelineArtifactPreviewValidated(selectedRunId, entry.path)
            if (!isCurrentRequest()) return
            updateRunDetailSession(selectedRunId, {
                artifactViewerPayload: payload,
                artifactViewerError: null,
                artifactViewerStatus: 'ready',
            })
        } catch (error) {
            if (!isCurrentRequest()) return
            logUnexpectedRunError(error)
            updateRunDetailSession(selectedRunId, {
                artifactViewerStatus: 'error',
                artifactViewerError: error instanceof ApiHttpError
                    ? artifactPreviewErrorFromResponse(error.status, error.detail)
                    : 'Unable to load artifact preview. Check your network/backend connection and retry.',
            })
        }
    }, [selectedRunId, updateRunDetailSession])

    const artifactDownloadHref = useCallback((artifactPath: string) => {
        if (!selectedRunId) {
            return ''
        }
        return pipelineArtifactHref(selectedRunId, artifactPath, true)
    }, [selectedRunId])

    return {
        artifactData: session.artifactData,
        artifactDownloadHref,
        artifactError: session.artifactError,
        artifactViewerError: session.artifactViewerError,
        artifactViewerPayload: session.artifactViewerPayload,
        artifactViewerStatus: session.artifactViewerStatus,
        checkpointData: session.checkpointData,
        checkpointError: session.checkpointError,
        checkpointStatus: session.checkpointStatus,
        contextCopyStatus: session.contextCopyStatus,
        contextData: session.contextData,
        contextError: session.contextError,
        contextSearchQuery: session.contextSearchQuery,
        contextStatus: session.contextStatus,
        fetchArtifacts,
        fetchCheckpoint,
        fetchContext,
        fetchResult,
        artifactStatus: session.artifactStatus,
        isArtifactLoading: session.artifactStatus === 'loading',
        isArtifactViewerLoading: session.artifactViewerStatus === 'loading',
        isCheckpointLoading: session.checkpointStatus === 'loading',
        isContextLoading: session.contextStatus === 'loading',
        isResultLoading: session.resultStatus === 'loading',
        pendingQuestionSnapshots: session.pendingQuestionSnapshots,
        questionsStatus: session.questionsStatus,
        selectedArtifactPath: session.selectedArtifactPath,
        setContextCopyStatus: (value: string) => {
            if (!selectedRunId) {
                return
            }
            updateRunDetailSession(selectedRunId, { contextCopyStatus: value })
        },
        setContextSearchQuery: (value: string) => {
            if (!selectedRunId) {
                return
            }
            updateRunDetailSession(selectedRunId, { contextSearchQuery: value })
        },
        resultData: session.resultData,
        resultError: session.resultError,
        resultStatus: session.resultStatus,
        viewArtifact,
    }
}
