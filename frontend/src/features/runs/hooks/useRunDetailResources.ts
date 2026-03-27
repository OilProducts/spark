import { useCallback, useEffect, useState } from 'react'

import {
    ApiHttpError,
    fetchPipelineArtifactPreviewValidated,
    fetchPipelineArtifactsValidated,
    fetchPipelineCheckpointValidated,
    fetchPipelineContextValidated,
    fetchPipelineQuestionsValidated,
    pipelineArtifactHref,
} from '@/lib/attractorClient'

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
    viewMode: string
}

export function useRunDetailResources({ selectedRunSummary, viewMode }: UseRunDetailResourcesArgs) {
    const [checkpointData, setCheckpointData] = useState<CheckpointResponse | null>(null)
    const [isCheckpointLoading, setIsCheckpointLoading] = useState(false)
    const [checkpointError, setCheckpointError] = useState<CheckpointErrorState | null>(null)
    const [contextData, setContextData] = useState<ContextResponse | null>(null)
    const [isContextLoading, setIsContextLoading] = useState(false)
    const [contextError, setContextError] = useState<ContextErrorState | null>(null)
    const [contextSearchQuery, setContextSearchQuery] = useState('')
    const [contextCopyStatus, setContextCopyStatus] = useState('')
    const [artifactData, setArtifactData] = useState<ArtifactListResponse | null>(null)
    const [isArtifactLoading, setIsArtifactLoading] = useState(false)
    const [artifactError, setArtifactError] = useState<ArtifactErrorState | null>(null)
    const [selectedArtifactPath, setSelectedArtifactPath] = useState<string | null>(null)
    const [artifactViewerPayload, setArtifactViewerPayload] = useState('')
    const [artifactViewerError, setArtifactViewerError] = useState<string | null>(null)
    const [isArtifactViewerLoading, setIsArtifactViewerLoading] = useState(false)
    const [pendingQuestionSnapshots, setPendingQuestionSnapshots] = useState<PendingQuestionSnapshot[]>([])

    const fetchCheckpoint = useCallback(async () => {
        if (!selectedRunSummary) {
            setCheckpointData(null)
            setCheckpointError(null)
            setIsCheckpointLoading(false)
            return
        }
        setIsCheckpointLoading(true)
        setCheckpointError(null)
        try {
            const payload = await fetchPipelineCheckpointValidated(selectedRunSummary.run_id) as CheckpointResponse
            setCheckpointData(payload)
        } catch (err) {
            logUnexpectedRunError(err)
            setCheckpointData(null)
            if (err instanceof ApiHttpError) {
                setCheckpointError(checkpointErrorFromResponse(err.status, err.detail))
                return
            }
            setCheckpointError({
                message: 'Unable to load checkpoint.',
                help: 'Check your network/backend connection and retry.',
            })
        } finally {
            setIsCheckpointLoading(false)
        }
    }, [selectedRunSummary])

    const fetchContext = useCallback(async () => {
        if (!selectedRunSummary) {
            setContextData(null)
            setContextError(null)
            setIsContextLoading(false)
            return
        }
        setIsContextLoading(true)
        setContextError(null)
        try {
            const payload = await fetchPipelineContextValidated(selectedRunSummary.run_id) as ContextResponse
            setContextData(payload)
        } catch (err) {
            logUnexpectedRunError(err)
            setContextData(null)
            if (err instanceof ApiHttpError) {
                setContextError(contextErrorFromResponse(err.status, err.detail))
                return
            }
            setContextError({
                message: 'Unable to load context.',
                help: 'Check your network/backend connection and retry.',
            })
        } finally {
            setIsContextLoading(false)
        }
    }, [selectedRunSummary])

    const fetchArtifacts = useCallback(async () => {
        if (!selectedRunSummary) {
            setArtifactData(null)
            setArtifactError(null)
            setIsArtifactLoading(false)
            return
        }
        setIsArtifactLoading(true)
        setArtifactError(null)
        try {
            const payload = await fetchPipelineArtifactsValidated(selectedRunSummary.run_id)
            setArtifactData(payload)
        } catch (err) {
            logUnexpectedRunError(err)
            setArtifactData(null)
            if (err instanceof ApiHttpError) {
                setArtifactError(artifactErrorFromResponse(err.status, err.detail))
                return
            }
            setArtifactError({
                message: 'Unable to load artifacts.',
                help: 'Check your network/backend connection and retry.',
            })
        } finally {
            setIsArtifactLoading(false)
        }
    }, [selectedRunSummary])

    const fetchPendingQuestions = useCallback(async () => {
        if (!selectedRunSummary) {
            setPendingQuestionSnapshots([])
            return
        }
        try {
            const payload = await fetchPipelineQuestionsValidated(selectedRunSummary.run_id)
            const rawQuestions = payload.questions
            if (!Array.isArray(rawQuestions)) {
                setPendingQuestionSnapshots([])
                return
            }
            const parsedQuestions = rawQuestions
                .map((question) => asPendingQuestionSnapshot(question))
                .filter((question): question is PendingQuestionSnapshot => question !== null)
            setPendingQuestionSnapshots(parsedQuestions)
        } catch (error) {
            logUnexpectedRunError(error)
            setPendingQuestionSnapshots([])
        }
    }, [selectedRunSummary])

    useEffect(() => {
        if (viewMode !== 'runs' || !selectedRunSummary) {
            setCheckpointData(null)
            setCheckpointError(null)
            setIsCheckpointLoading(false)
            return
        }
        void fetchCheckpoint()
    }, [fetchCheckpoint, selectedRunSummary, viewMode])

    useEffect(() => {
        if (viewMode !== 'runs' || !selectedRunSummary) {
            setContextData(null)
            setContextError(null)
            setContextSearchQuery('')
            setContextCopyStatus('')
            setIsContextLoading(false)
            return
        }
        void fetchContext()
    }, [fetchContext, selectedRunSummary, viewMode])

    useEffect(() => {
        if (viewMode !== 'runs' || !selectedRunSummary) {
            setArtifactData(null)
            setArtifactError(null)
            setSelectedArtifactPath(null)
            setArtifactViewerPayload('')
            setArtifactViewerError(null)
            setIsArtifactLoading(false)
            setIsArtifactViewerLoading(false)
            return
        }
        void fetchArtifacts()
    }, [fetchArtifacts, selectedRunSummary, viewMode])

    useEffect(() => {
        if (viewMode !== 'runs' || !selectedRunSummary) {
            setPendingQuestionSnapshots([])
            return
        }
        void fetchPendingQuestions()
    }, [fetchPendingQuestions, selectedRunSummary, viewMode])

    const viewArtifact = useCallback(async (entry: { path: string; viewable: boolean }) => {
        if (!selectedRunSummary) {
            return
        }
        setSelectedArtifactPath(entry.path)
        setArtifactViewerPayload('')
        setArtifactViewerError(null)
        if (!entry.viewable) {
            setArtifactViewerError('Preview unavailable for this artifact type. Use download action.')
            return
        }
        setIsArtifactViewerLoading(true)
        try {
            const payload = await fetchPipelineArtifactPreviewValidated(selectedRunSummary.run_id, entry.path)
            setArtifactViewerPayload(payload)
        } catch (error) {
            logUnexpectedRunError(error)
            if (error instanceof ApiHttpError) {
                setArtifactViewerError(artifactPreviewErrorFromResponse(error.status, error.detail))
                return
            }
            setArtifactViewerError('Unable to load artifact preview. Check your network/backend connection and retry.')
        } finally {
            setIsArtifactViewerLoading(false)
        }
    }, [selectedRunSummary])

    const artifactDownloadHref = useCallback((artifactPath: string) => {
        if (!selectedRunSummary) {
            return ''
        }
        return pipelineArtifactHref(selectedRunSummary.run_id, artifactPath, true)
    }, [selectedRunSummary])

    return {
        artifactData,
        artifactDownloadHref,
        artifactError,
        artifactViewerError,
        artifactViewerPayload,
        checkpointData,
        checkpointError,
        contextCopyStatus,
        contextData,
        contextError,
        contextSearchQuery,
        fetchArtifacts,
        fetchCheckpoint,
        fetchContext,
        isArtifactLoading,
        isArtifactViewerLoading,
        isCheckpointLoading,
        isContextLoading,
        pendingQuestionSnapshots,
        selectedArtifactPath,
        setContextCopyStatus,
        setContextSearchQuery,
        viewArtifact,
    }
}
