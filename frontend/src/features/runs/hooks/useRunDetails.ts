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
        isArtifactLoading,
        isArtifactViewerLoading,
        isCheckpointLoading,
        isContextLoading,
        pendingQuestionSnapshots,
        questionsStatus,
        selectedArtifactPath,
        setContextCopyStatus,
        setContextSearchQuery,
        viewArtifact,
    } = useRunDetailResources({
        selectedRunSummary,
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
        if (!selectedRunSummary) {
            return ''
        }
        return buildContextExportPayload(
            selectedRunSummary.run_id,
            filteredContextRows.map((row) => ({ key: row.key, value: row.rawValue })),
        )
    }, [filteredContextRows, selectedRunSummary])
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
        filteredContextRows,
        isArtifactLoading,
        isArtifactViewerLoading,
        isCheckpointLoading,
        isContextLoading,
        missingCoreArtifacts,
        pendingQuestionSnapshots,
        questionsStatus,
        selectedArtifactEntry,
        setContextCopyStatus,
        setContextSearchQuery,
        showPartialRunArtifactNote,
        viewArtifact,
        copyContextToClipboard,
    }
}
