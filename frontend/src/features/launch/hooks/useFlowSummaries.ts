import { useEffect, useMemo, useState } from 'react'

import { flowTitle } from '@/features/runs/model/runOverviewModel'
import { useRunsTransportReconnectSignal } from '@/features/runs/services/runsTransportReconnect'
import { fetchWorkspaceFlowListValidated, type WorkspaceFlowResponse } from '@/lib/api/flowsApi'

/**
 * The flows a project can run, with their titles and descriptions; null while loading. Reloads when Spark reconnects.
 * A flow that doesn't load is titled from its name, as its launch form titles it, not by its bare file name.
 */
export function useFlowSummaries(projectPath: string | null) {
    const [loaded, setLoaded] = useState<{ projectPath: string | null; flows: WorkspaceFlowResponse[]; error: string | null } | null>(null)
    const [attempt, setAttempt] = useState(0)
    const reconnectSignal = useRunsTransportReconnectSignal()
    useEffect(() => {
        let cancelled = false
        fetchWorkspaceFlowListValidated(projectPath)
            .then((flows) => flows.map((flow) => (flow.error ? { ...flow, title: flowTitle(flow.name) } : flow)))
            .then((flows) => { if (!cancelled) setLoaded({ projectPath, flows, error: null }) })
            .catch((error: unknown) => {
                if (!cancelled) setLoaded({ projectPath, flows: [], error: error instanceof Error ? error.message : 'The flows could not be loaded.' })
            })
        return () => { cancelled = true }
    }, [projectPath, attempt, reconnectSignal])
    const current = loaded?.projectPath === projectPath ? loaded : null
    return useMemo(() => current && { ...current, retry: () => setAttempt((value) => value + 1) }, [current])
}
