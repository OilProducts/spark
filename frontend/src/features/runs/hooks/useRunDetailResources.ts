import { useCallback, useEffect, useMemo } from 'react'

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
    const runDetailSessionsByRunId = useStore((state) => state.runDetailSessionsByRunId)
    const updateRunDetailSession = useStore((state) => state.updateRunDetailSession)
    const session = useMemo(() => {
        if (!selectedRunId) {
            return DEFAULT_RUN_DETAIL_SESSION
        }
        return {
            ...DEFAULT_RUN_DETAIL_SESSION,
            ...(runDetailSessionsByRunId[selectedRunId] ?? {}),
        }
    }, [runDetailSessionsByRunId, selectedRunId])

    const fetchCheckpoint = useCallback(async () => {
        if (!selectedRunId) {
            return
        }
        updateRunDetailSession(selectedRunId, {
            checkpointStatus: 'loading',
            checkpointError: null,
        })
        try {
            const payload = await fetchPipelineCheckpointValidated(selectedRunId) as CheckpointResponse
            updateRunDetailSession(selectedRunId, {
                checkpointData: payload,
                checkpointStatus: 'ready',
                checkpointError: null,
            })
        } catch (err) {
            logUnexpectedRunError(err)
            updateRunDetailSession(selectedRunId, {
                checkpointData: null,
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
        updateRunDetailSession(selectedRunId, {
            contextStatus: 'loading',
            contextError: null,
        })
        try {
            const payload = await fetchPipelineContextValidated(selectedRunId) as ContextResponse
            updateRunDetailSession(selectedRunId, {
                contextData: payload,
                contextStatus: 'ready',
                contextError: null,
            })
        } catch (err) {
            logUnexpectedRunError(err)
            updateRunDetailSession(selectedRunId, {
                contextData: null,
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
        updateRunDetailSession(selectedRunId, {
            artifactStatus: 'loading',
            artifactError: null,
        })
        try {
            const payload = await fetchPipelineArtifactsValidated(selectedRunId)
            updateRunDetailSession(selectedRunId, {
                artifactData: payload,
                artifactStatus: 'ready',
                artifactError: null,
            })
        } catch (err) {
            logUnexpectedRunError(err)
            updateRunDetailSession(selectedRunId, {
                artifactData: null,
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
        updateRunDetailSession(selectedRunId, {
            resultStatus: 'loading',
            resultError: null,
        })
        try {
            const payload = await fetchPipelineResultValidated(selectedRunId)
            updateRunDetailSession(selectedRunId, {
                resultData: payload,
                resultStatus: 'ready',
                resultError: null,
            })
        } catch (err) {
            logUnexpectedRunError(err)
            updateRunDetailSession(selectedRunId, {
                resultData: null,
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
        updateRunDetailSession(selectedRunId, {
            questionsStatus: 'loading',
        })
        try {
            const payload = await fetchPipelineQuestionsValidated(selectedRunId)
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
            logUnexpectedRunError(error)
            updateRunDetailSession(selectedRunId, {
                pendingQuestionSnapshots: [],
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
    }, [fetchArtifacts, fetchCheckpoint, fetchContext, fetchPendingQuestions, fetchResult, manageSync, selectedRunId])

    const viewArtifact = useCallback(async (entry: { path: string; viewable: boolean }) => {
        if (!selectedRunId) {
            return
        }
        updateRunDetailSession(selectedRunId, {
            selectedArtifactPath: entry.path,
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
            updateRunDetailSession(selectedRunId, {
                artifactViewerPayload: payload,
                artifactViewerError: null,
                artifactViewerStatus: 'ready',
            })
        } catch (error) {
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
