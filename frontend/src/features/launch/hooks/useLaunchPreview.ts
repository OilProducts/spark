import { useEffect, useMemo, useState } from 'react'

import { buildHydratedFlowGraph, normalizeLegacyDot, type HydratedFlowGraph } from '@/features/workflow-canvas'
import {
    fetchFlowPayloadValidated,
    fetchPipelineGraphPreviewValidated,
    fetchPreviewValidated,
} from '@/lib/attractorClient'
import { isAbortError } from '@/lib/abortError'
import { useStore } from '@/store'
import type { DiagnosticEntry } from '@/state/store-types'

import type { LaunchPreviewSource } from '../model/launchTypes'

const unexpectedPreviewLoadMessage = 'Unable to load flow preview for launch inputs.'

export interface LaunchPreviewState {
    isLoadingPreview: boolean
    previewLoadError: string | null
    hydratedGraph: HydratedFlowGraph | null
    diagnostics: DiagnosticEntry[]
    hasValidationErrors: boolean
    graphAttrs: Record<string, unknown>
}

export function useLaunchPreview(
    source: LaunchPreviewSource | null,
    expandChildFlows = false,
): LaunchPreviewState {
    const uiDefaults = useStore((state) => state.uiDefaults)
    const [isLoadingPreview, setIsLoadingPreview] = useState(false)
    const [previewLoadError, setPreviewLoadError] = useState<string | null>(null)
    const [hydratedGraph, setHydratedGraph] = useState<HydratedFlowGraph | null>(null)
    const [diagnostics, setDiagnostics] = useState<DiagnosticEntry[]>([])

    const sourceKey = source
        ? source.kind === 'flow'
            ? `flow:${source.flowName}`
            : `runSnapshot:${source.runId}:${source.displayName ?? ''}`
        : null

    useEffect(() => {
        if (!source) {
            setPreviewLoadError(null)
            setHydratedGraph(null)
            setDiagnostics([])
            setIsLoadingPreview(false)
            return
        }

        const loadAbort = new AbortController()
        let cancelled = false

        const loadPreview = async () => {
            setIsLoadingPreview(true)
            setPreviewLoadError(null)
            try {
                let preview
                let hydrated: HydratedFlowGraph | null = null

                if (source.kind === 'runSnapshot') {
                    preview = await fetchPipelineGraphPreviewValidated(
                        source.runId,
                        { signal: loadAbort.signal },
                        { expandChildren: expandChildFlows },
                    )
                    if (cancelled) {
                        return
                    }
                    hydrated = buildHydratedFlowGraph(
                        source.displayName || source.runId,
                        preview,
                        uiDefaults,
                        undefined,
                        { expandChildren: expandChildFlows },
                    )
                } else {
                    const payload = await fetchFlowPayloadValidated(source.flowName, { signal: loadAbort.signal })
                    if (cancelled) {
                        return
                    }

                    const normalizedContent = normalizeLegacyDot(payload.content)
                    preview = await fetchPreviewValidated(
                        normalizedContent,
                        { signal: loadAbort.signal },
                        {
                            flowName: source.flowName,
                            expandChildren: expandChildFlows,
                        },
                    )
                    if (cancelled) {
                        return
                    }

                    hydrated = buildHydratedFlowGraph(
                        source.flowName,
                        preview,
                        uiDefaults,
                        normalizedContent,
                        { expandChildren: expandChildFlows },
                    )
                }

                if (cancelled) {
                    return
                }

                setDiagnostics(preview?.diagnostics ?? [])
                setHydratedGraph(hydrated)
            } catch (error) {
                if (loadAbort.signal.aborted || isAbortError(error)) {
                    return
                }
                console.error(error)
                setHydratedGraph(null)
                setDiagnostics([])
                setPreviewLoadError(unexpectedPreviewLoadMessage)
            } finally {
                if (!cancelled) {
                    setIsLoadingPreview(false)
                }
            }
        }

        void loadPreview()

        return () => {
            cancelled = true
            loadAbort.abort()
        }
        // eslint-disable-next-line react-hooks/exhaustive-deps -- sourceKey captures the source identity
    }, [sourceKey, expandChildFlows, uiDefaults])

    const hasValidationErrors = useMemo(
        () => diagnostics.some((diagnostic) => diagnostic.severity === 'error'),
        [diagnostics],
    )
    const graphAttrs = hydratedGraph?.graphAttrs ?? {}

    return {
        isLoadingPreview,
        previewLoadError,
        hydratedGraph,
        diagnostics,
        hasValidationErrors,
        graphAttrs,
    }
}
