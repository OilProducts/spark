import { useCallback, useEffect, useMemo } from 'react'

import {
    ApiHttpError,
    fetchPipelineArtifactPreviewValidated,
    fetchPipelineArtifactsValidated,
    fetchPipelineCheckpointValidated,
    fetchPipelineContextValidated,
    fetchPipelineQuestionsValidated,
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
    RunRecord,
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
    selectedRunSummary: RunRecord | null
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
    selectedRunSummary,
    manageSync = true,
}: UseRunDetailResourcesArgs) {
    const runDetailSessionsByRunId = useStore((state) => state.runDetailSessionsByRunId)
    const updateRunDetailSession = useStore((state) => state.updateRunDetailSession)
    const selectedRunId = selectedRunSummary?.run_id ?? null
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
        if (!selectedRunSummary) {
            return
        }
        updateRunDetailSession(selectedRunSummary.run_id, {
            checkpointStatus: 'loading',
            checkpointError: null,
        })
        try {
            const payload = await fetchPipelineCheckpointValidated(selectedRunSummary.run_id) as CheckpointResponse
            updateRunDetailSession(selectedRunSummary.run_id, {
                checkpointData: payload,
                checkpointStatus: 'ready',
                checkpointError: null,
            })
        } catch (err) {
            logUnexpectedRunError(err)
            updateRunDetailSession(selectedRunSummary.run_id, {
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
    }, [selectedRunSummary, updateRunDetailSession])

    const fetchContext = useCallback(async () => {
        if (!selectedRunSummary) {
            return
        }
        updateRunDetailSession(selectedRunSummary.run_id, {
            contextStatus: 'loading',
            contextError: null,
        })
        try {
            const payload = await fetchPipelineContextValidated(selectedRunSummary.run_id) as ContextResponse
            updateRunDetailSession(selectedRunSummary.run_id, {
                contextData: payload,
                contextStatus: 'ready',
                contextError: null,
            })
        } catch (err) {
            logUnexpectedRunError(err)
            updateRunDetailSession(selectedRunSummary.run_id, {
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
    }, [selectedRunSummary, updateRunDetailSession])

    const fetchArtifacts = useCallback(async () => {
        if (!selectedRunSummary) {
            return
        }
        updateRunDetailSession(selectedRunSummary.run_id, {
            artifactStatus: 'loading',
            artifactError: null,
        })
        try {
            const payload = await fetchPipelineArtifactsValidated(selectedRunSummary.run_id)
            updateRunDetailSession(selectedRunSummary.run_id, {
                artifactData: payload,
                artifactStatus: 'ready',
                artifactError: null,
            })
        } catch (err) {
            logUnexpectedRunError(err)
            updateRunDetailSession(selectedRunSummary.run_id, {
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
    }, [selectedRunSummary, updateRunDetailSession])

    const fetchPendingQuestions = useCallback(async () => {
        if (!selectedRunSummary) {
            return
        }
        updateRunDetailSession(selectedRunSummary.run_id, {
            questionsStatus: 'loading',
        })
        try {
            const payload = await fetchPipelineQuestionsValidated(selectedRunSummary.run_id)
            const rawQuestions = payload.questions
            const parsedQuestions = Array.isArray(rawQuestions)
                ? rawQuestions
                    .map((question) => asPendingQuestionSnapshot(question))
                    .filter((question): question is PendingQuestionSnapshot => question !== null)
                : []
            updateRunDetailSession(selectedRunSummary.run_id, {
                pendingQuestionSnapshots: parsedQuestions,
                questionsStatus: 'ready',
            })
        } catch (error) {
            logUnexpectedRunError(error)
            updateRunDetailSession(selectedRunSummary.run_id, {
                pendingQuestionSnapshots: [],
                questionsStatus: 'error',
            })
        }
    }, [selectedRunSummary, updateRunDetailSession])

    useEffect(() => {
        if (!manageSync || !selectedRunSummary) {
            return
        }
        void fetchCheckpoint()
        void fetchContext()
        void fetchArtifacts()
        void fetchPendingQuestions()
    }, [fetchArtifacts, fetchCheckpoint, fetchContext, fetchPendingQuestions, manageSync, selectedRunSummary])

    const viewArtifact = useCallback(async (entry: { path: string; viewable: boolean }) => {
        if (!selectedRunSummary) {
            return
        }
        updateRunDetailSession(selectedRunSummary.run_id, {
            selectedArtifactPath: entry.path,
            artifactViewerPayload: '',
            artifactViewerError: null,
        })
        if (!entry.viewable) {
            updateRunDetailSession(selectedRunSummary.run_id, {
                artifactViewerStatus: 'error',
                artifactViewerError: 'Preview unavailable for this artifact type. Use download action.',
            })
            return
        }
        updateRunDetailSession(selectedRunSummary.run_id, {
            artifactViewerStatus: 'loading',
        })
        try {
            const payload = await fetchPipelineArtifactPreviewValidated(selectedRunSummary.run_id, entry.path)
            updateRunDetailSession(selectedRunSummary.run_id, {
                artifactViewerPayload: payload,
                artifactViewerError: null,
                artifactViewerStatus: 'ready',
            })
        } catch (error) {
            logUnexpectedRunError(error)
            updateRunDetailSession(selectedRunSummary.run_id, {
                artifactViewerStatus: 'error',
                artifactViewerError: error instanceof ApiHttpError
                    ? artifactPreviewErrorFromResponse(error.status, error.detail)
                    : 'Unable to load artifact preview. Check your network/backend connection and retry.',
            })
        }
    }, [selectedRunSummary, updateRunDetailSession])

    const artifactDownloadHref = useCallback((artifactPath: string) => {
        if (!selectedRunSummary) {
            return ''
        }
        return pipelineArtifactHref(selectedRunSummary.run_id, artifactPath, true)
    }, [selectedRunSummary])

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
        artifactStatus: session.artifactStatus,
        isArtifactLoading: session.artifactStatus === 'loading',
        isArtifactViewerLoading: session.artifactViewerStatus === 'loading',
        isCheckpointLoading: session.checkpointStatus === 'loading',
        isContextLoading: session.contextStatus === 'loading',
        pendingQuestionSnapshots: session.pendingQuestionSnapshots,
        questionsStatus: session.questionsStatus,
        selectedArtifactPath: session.selectedArtifactPath,
        setContextCopyStatus: (value: string) => {
            if (!selectedRunSummary) {
                return
            }
            updateRunDetailSession(selectedRunSummary.run_id, { contextCopyStatus: value })
        },
        setContextSearchQuery: (value: string) => {
            if (!selectedRunSummary) {
                return
            }
            updateRunDetailSession(selectedRunSummary.run_id, { contextSearchQuery: value })
        },
        viewArtifact,
    }
}
