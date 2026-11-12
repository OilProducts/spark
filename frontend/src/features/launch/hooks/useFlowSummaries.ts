import { useEffect, useState } from 'react'

import { fetchWorkspaceFlowListValidated, type WorkspaceFlowResponse } from '@/lib/api/flowsApi'

/** The flows a project can run, with their titles and descriptions; null while loading. */
export function useFlowSummaries(projectPath: string | null) {
    const [loaded, setLoaded] = useState<{ projectPath: string | null; flows: WorkspaceFlowResponse[]; error: string | null } | null>(null)
    useEffect(() => {
        let cancelled = false
        fetchWorkspaceFlowListValidated(projectPath)
            .then((flows) => { if (!cancelled) setLoaded({ projectPath, flows, error: null }) })
            .catch((error: unknown) => {
                if (!cancelled) setLoaded({ projectPath, flows: [], error: error instanceof Error ? error.message : 'The flows could not be loaded.' })
            })
        return () => { cancelled = true }
    }, [projectPath])
    return loaded?.projectPath === projectPath ? loaded : null
}
