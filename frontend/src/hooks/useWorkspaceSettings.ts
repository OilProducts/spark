import { useEffect, useState } from 'react'
import { fetchWorkspaceSettingsValidated, type WorkspaceSettingsResponse } from '@/lib/workspaceClient'

export function useWorkspaceSettings(): {
    workspaceSettings: WorkspaceSettingsResponse | null
    settingsError: string | null
} {
    const [workspaceSettings, setWorkspaceSettings] = useState<WorkspaceSettingsResponse | null>(null)
    const [settingsError, setSettingsError] = useState<string | null>(null)

    useEffect(() => {
        void fetchWorkspaceSettingsValidated()
            .then((payload) => {
                setWorkspaceSettings(payload)
                setSettingsError(null)
            })
            .catch((error: unknown) => {
                setWorkspaceSettings(null)
                setSettingsError(error instanceof Error ? error.message : 'Unable to load workspace settings.')
            })
    }, [])

    return { workspaceSettings, settingsError }
}
