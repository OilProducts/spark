import { useCallback, useMemo } from 'react'
import type {
    RunRecord,
} from '../model/shared'
import {
    buildArtifactDerivedState,
    buildCheckpointSummary,
    buildContextExportPayload,
    buildContextRows,
    buildDegradedDetailPanels,
    filterContextRows,
} from '../model/runDetailsModel'
import { useRunDetailResources } from './useRunDetailResources'

type UseRunDetailsArgs = {
    selectedRunSummary: RunRecord | null
    manageSync?: boolean
}

export function useRunDetails({
    selectedRunSummary,
    manageSync = true,
}: UseRunDetailsArgs) {
    const selectedRunId = selectedRunSummary?.run_id ?? null
    const {
        artifactData,
        artifactDownloadHref,
        artifactError,
        artifactStatus,
        artifactViewerError,
        artifactViewerPayload,
        artifactViewerStatus,
        checkpointData,
        checkpointError,
        checkpointStatus,
        contextCopyStatus,
        contextData,
        contextError,
        contextSearchQuery,
        contextStatus,
        fetchArtifacts,
        fetchCheckpoint,
        fetchContext,
        fetchResult,
        isArtifactLoading,
        isArtifactViewerLoading,
        isCheckpointLoading,
        isContextLoading,
        isResultLoading,
        pendingQuestionSnapshots,
        questionsStatus,
        resultData,
        resultError,
        resultStatus,
        selectedArtifactPath,
        setContextCopyStatus,
        setContextSearchQuery,
        viewArtifact,
    } = useRunDetailResources({
        selectedRunId,
        manageSync,
    })
    const {
        checkpointCompletedNodes,
        checkpointCurrentNode,
        checkpointRetryCounters,
    } = useMemo(() => buildCheckpointSummary(checkpointData), [checkpointData])
    const contextRows = useMemo(() => buildContextRows(contextData), [contextData])
    const filteredContextRows = useMemo(() => {
        return filterContextRows(contextRows, contextSearchQuery)
    }, [contextRows, contextSearchQuery])
    const contextExportPayload = useMemo(() => {
        if (!selectedRunId) {
            return ''
        }
        return buildContextExportPayload(
            selectedRunId,
            filteredContextRows.map((row) => ({ key: row.key, value: row.rawValue })),
        )
    }, [filteredContextRows, selectedRunId])
    const contextExportHref = useMemo(() => {
        if (!contextExportPayload) {
            return ''
        }
        return `data:application/json;charset=utf-8,${encodeURIComponent(contextExportPayload)}`
    }, [contextExportPayload])
    const copyContextToClipboard = useCallback(async () => {
        if (!contextExportPayload || filteredContextRows.length === 0) {
            setContextCopyStatus('No context entries available to copy.')
            return
        }
        try {
            await window.navigator.clipboard.writeText(contextExportPayload)
            setContextCopyStatus('Filtered context copied.')
        } catch (error) {
            console.error(error)
            setContextCopyStatus('Copy failed. Clipboard access is unavailable.')
        }
    }, [contextExportPayload, filteredContextRows, setContextCopyStatus])
    const {
        artifactEntries,
        missingCoreArtifacts,
        selectedArtifactEntry,
        showPartialRunArtifactNote,
    } = useMemo(
        () => buildArtifactDerivedState(artifactData, selectedArtifactPath),
        [artifactData, selectedArtifactPath],
    )

    const degradedDetailPanels = useMemo(() => {
        return buildDegradedDetailPanels({
            checkpointError,
            contextError,
            artifactError,
        })
    }, [artifactError, checkpointError, contextError])

    return {
        artifactDownloadHref,
        artifactEntries,
        artifactError,
        artifactStatus,
        artifactViewerError,
        artifactViewerPayload,
        artifactViewerStatus,
        checkpointCompletedNodes,
        checkpointCurrentNode,
        checkpointData,
        checkpointError,
        checkpointStatus,
        checkpointRetryCounters,
        contextCopyStatus,
        contextError,
        contextExportHref,
        contextSearchQuery,
        contextStatus,
        degradedDetailPanels,
        fetchArtifacts,
        fetchCheckpoint,
        fetchContext,
        fetchResult,
        filteredContextRows,
        isArtifactLoading,
        isArtifactViewerLoading,
        isCheckpointLoading,
        isContextLoading,
        isResultLoading,
        missingCoreArtifacts,
        pendingQuestionSnapshots,
        questionsStatus,
        resultData,
        resultError,
        resultStatus,
        selectedArtifactEntry,
        setContextCopyStatus,
        setContextSearchQuery,
        showPartialRunArtifactNote,
        viewArtifact,
        copyContextToClipboard,
    }
}
