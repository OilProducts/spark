import { useEffect, useState } from 'react'

import { buildHydratedFlowGraph, normalizeLegacyDot } from '@/features/workflow-canvas'
import { isAbortError } from '@/lib/api/shared'
import { useStore } from '@/store'

import { loadExecutionFlowPayload, loadExecutionFlowPreview } from '../services/executionPreviewTransport'

const unexpectedPreviewLoadMessage = 'Unable to load flow preview for launch inputs.'

export function useExecutionLaunchPreview(executionFlow: string | null) {
    const uiDefaults = useStore((state) => state.uiDefaults)
    const replaceExecutionGraphAttrs = useStore((state) => state.replaceExecutionGraphAttrs)
    const setExecutionDiagnostics = useStore((state) => state.setExecutionDiagnostics)
    const clearExecutionDiagnostics = useStore((state) => state.clearExecutionDiagnostics)
    const [isLoadingPreview, setIsLoadingPreview] = useState(false)
    const [previewLoadError, setPreviewLoadError] = useState<string | null>(null)

    useEffect(() => {
        if (!executionFlow) {
            replaceExecutionGraphAttrs({})
            clearExecutionDiagnostics()
            setPreviewLoadError(null)
            setIsLoadingPreview(false)
            return
        }

        const loadAbort = new AbortController()
        let cancelled = false

        const loadPreview = async () => {
            setIsLoadingPreview(true)
            setPreviewLoadError(null)
            try {
                const payload = await loadExecutionFlowPayload(executionFlow, { signal: loadAbort.signal })
                if (cancelled) {
                    return
                }

                const normalizedContent = normalizeLegacyDot(payload.content)
                const preview = await loadExecutionFlowPreview(normalizedContent, { signal: loadAbort.signal })
                if (cancelled) {
                    return
                }

                if (preview.diagnostics) {
                    setExecutionDiagnostics(preview.diagnostics)
                } else {
                    clearExecutionDiagnostics()
                }

                const hydratedGraph = buildHydratedFlowGraph(
                    executionFlow,
                    preview,
                    uiDefaults,
                    normalizedContent,
                )
                replaceExecutionGraphAttrs(hydratedGraph?.graphAttrs ?? {})
            } catch (error) {
                if (loadAbort.signal.aborted || isAbortError(error)) {
                    return
                }
                console.error(error)
                replaceExecutionGraphAttrs({})
                clearExecutionDiagnostics()
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
    }, [
        clearExecutionDiagnostics,
        executionFlow,
        replaceExecutionGraphAttrs,
        setExecutionDiagnostics,
        uiDefaults,
    ])

    return {
        isLoadingPreview,
        previewLoadError,
    }
}
