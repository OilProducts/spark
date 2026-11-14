import { useEffect, useState } from 'react'

import { ApiHttpError } from '@/lib/attractorClient'
import {
    fetchWorkspaceFlowValidated,
    type WorkspaceFlowResponse,
} from '@/lib/workspaceClient'

export type FlowLaunchMetadata = WorkspaceFlowResponse & {
    /** Set when the request failed (the network or the server), which says nothing of the flow: ask again. */
    retry?: () => void
}

/**
 * A flow's description, null while it loads. A flow that can't be read comes back as its name
 * and `error`, so callers can tell a settled failure from a load still under way.
 */
export function useFlowLaunchMetadata(flowName: string | null, projectPath: string | null): FlowLaunchMetadata | null {
    const [flowMetadata, setFlowMetadata] = useState<FlowLaunchMetadata | null>(null)
    const [attempt, setAttempt] = useState(0)

    useEffect(() => {
        // Another flow or project's description isn't this one's.
        setFlowMetadata(null)
        if (!flowName) {
            return
        }
        let cancelled = false
        fetchWorkspaceFlowValidated(flowName, projectPath)
            .then((payload) => {
                if (cancelled) {
                    return
                }
                setFlowMetadata(payload)
            })
            .catch((error: unknown) => {
                if (cancelled) {
                    return
                }
                // The server judging the flow unreadable is settled; a request the network or server failed can be tried again.
                const settled = !(error instanceof TypeError) && !(error instanceof ApiHttpError && error.status >= 500)
                setFlowMetadata({
                    name: flowName,
                    title: '',
                    description: '',
                    error: error instanceof Error ? error.message : 'This flow could not be read.',
                    ...(settled ? {} : { retry: () => setAttempt((count) => count + 1) }),
                })
            })
        return () => {
            cancelled = true
        }
    }, [flowName, projectPath, attempt])

    return flowMetadata
}
