import { useEffect, useState } from 'react'
import {
    fetchWorkspaceFlowValidated,
    fetchWorkspaceSettingsValidated,
    type WorkspaceFlowResponse,
    type WorkspaceSettingsResponse,
} from '@/lib/workspaceClient'

export function useExecutionWorkspaceMetadata(executionFlowName: string | null, enabled: boolean): {
    workspaceSettings: WorkspaceSettingsResponse | null
    workspaceSettingsError: string | null
    workspaceFlowMetadata: WorkspaceFlowResponse | null
} {
    const [workspaceSettings, setWorkspaceSettings] = useState<WorkspaceSettingsResponse | null>(null)
    const [workspaceSettingsError, setWorkspaceSettingsError] = useState<string | null>(null)
    const [workspaceFlowMetadata, setWorkspaceFlowMetadata] = useState<WorkspaceFlowResponse | null>(null)

    useEffect(() => {
        if (!executionFlowName || !enabled) {
            setWorkspaceSettings(null)
            setWorkspaceSettingsError(null)
            return
        }
        let cancelled = false
        fetchWorkspaceSettingsValidated()
            .then((payload) => {
                if (cancelled) {
                    return
                }
                setWorkspaceSettings(payload)
                setWorkspaceSettingsError(null)
            })
            .catch((error: unknown) => {
                if (cancelled) {
                    return
                }
                setWorkspaceSettings(null)
                setWorkspaceSettingsError(error instanceof Error ? error.message : 'Unable to load execution profiles.')
            })
        return () => {
            cancelled = true
        }
    }, [executionFlowName, enabled])

    useEffect(() => {
        if (!executionFlowName || !enabled) {
            setWorkspaceFlowMetadata(null)
            return
        }
        let cancelled = false
        fetchWorkspaceFlowValidated(executionFlowName)
            .then((payload) => {
                if (cancelled) {
                    return
                }
                setWorkspaceFlowMetadata(payload)
            })
            .catch(() => {
                if (cancelled) {
                    return
                }
                setWorkspaceFlowMetadata(null)
            })
        return () => {
            cancelled = true
        }
    }, [executionFlowName, enabled])

    return { workspaceSettings, workspaceSettingsError, workspaceFlowMetadata }
}
