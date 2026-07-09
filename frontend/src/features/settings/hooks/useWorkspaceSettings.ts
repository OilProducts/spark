import { useEffect, useState } from 'react'

import {
    fetchWorkspaceSettingsValidated,
    type WorkspaceSettingsResponse,
} from '@/lib/workspaceClient'

export function useWorkspaceSettings(): {
    workspaceSettings: WorkspaceSettingsResponse | null
    settingsError: string | null
} {
    const [workspaceSettings, setWorkspaceSettings] = useState<WorkspaceSettingsResponse | null>(null)
    const [settingsError, setSettingsError] = useState<string | null>(null)

    useEffect(() => {
        let cancelled = false
        fetchWorkspaceSettingsValidated()
            .then((payload) => {
                if (cancelled) {
                    return
                }
                setWorkspaceSettings(payload)
                setSettingsError(null)
            })
            .catch((error: unknown) => {
                if (cancelled) {
                    return
                }
                setWorkspaceSettings(null)
                setSettingsError(error instanceof Error ? error.message : 'Unable to load workspace settings.')
            })
        return () => {
            cancelled = true
        }
    }, [])

    return { workspaceSettings, settingsError }
}
