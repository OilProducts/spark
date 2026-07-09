import { useEffect, useState } from 'react'

import {
    fetchWorkspaceFlowValidated,
    type WorkspaceFlowResponse,
} from '@/lib/workspaceClient'

export function useFlowLaunchMetadata(flowName: string | null): WorkspaceFlowResponse | null {
    const [flowMetadata, setFlowMetadata] = useState<WorkspaceFlowResponse | null>(null)

    useEffect(() => {
        if (!flowName) {
            setFlowMetadata(null)
            return
        }
        let cancelled = false
        fetchWorkspaceFlowValidated(flowName)
            .then((payload) => {
                if (cancelled) {
                    return
                }
                setFlowMetadata(payload)
            })
            .catch(() => {
                if (cancelled) {
                    return
                }
                setFlowMetadata(null)
            })
        return () => {
            cancelled = true
        }
    }, [flowName])

    return flowMetadata
}
