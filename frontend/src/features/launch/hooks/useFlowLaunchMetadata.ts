import { useEffect, useState } from 'react'

import {
    fetchWorkspaceFlowValidated,
    type WorkspaceFlowResponse,
} from '@/lib/workspaceClient'

/**
 * A flow's description, null while it loads. A flow that can't be read comes back as its name
 * and `error`, so callers can tell a settled failure from a load still under way.
 */
export function useFlowLaunchMetadata(flowName: string | null, projectPath: string | null): WorkspaceFlowResponse | null {
    const [flowMetadata, setFlowMetadata] = useState<WorkspaceFlowResponse | null>(null)

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
                setFlowMetadata({ name: flowName, title: '', description: '', error: error instanceof Error ? error.message : 'This flow could not be read.' })
            })
        return () => {
            cancelled = true
        }
    }, [flowName, projectPath])

    return flowMetadata
}
