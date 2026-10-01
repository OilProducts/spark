import { useEffect, useState } from 'react'

import {
    fetchWorkspaceSettingsValidated,
    type ExecutionPlacementProfile,
    type WorkspaceSettingsResponse,
} from '@/lib/workspaceClient'
import { useStore } from '@/store'

export interface ExecutionPlacementState {
    selectedProfileId: string
    setSelectedProfileId: (profileId: string) => void
    effectiveProfileId: string
    effectiveProfile: ExecutionPlacementProfile | null
    enabledProfiles: ExecutionPlacementProfile[]
    projectDefaultProfileId: string | null
    validationMessage: string | null
    statusMessage: string | null
}

/** The execution profile a launch in `projectPath` uses, defaulting to that project's own. */
export function useExecutionPlacement(enabled: boolean, projectPath: string | null): ExecutionPlacementState {
    const projectRegistry = useStore((state) => state.projectRegistry)
    const [selectedProfileId, setSelectedProfileId] = useState('')
    const [workspaceSettings, setWorkspaceSettings] = useState<WorkspaceSettingsResponse | null>(null)
    const [workspaceSettingsError, setWorkspaceSettingsError] = useState<string | null>(null)

    useEffect(() => {
        if (!enabled) {
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
    }, [enabled])

    const executionPlacement = workspaceSettings?.execution_placement ?? null
    const projectDefaultProfileId = projectPath
        ? projectRegistry[projectPath]?.executionProfileId || null
        : null
    const effectiveProfileId = selectedProfileId
        || projectDefaultProfileId
        || executionPlacement?.default_execution_profile_id
        || 'native'
    const effectiveProfile = executionPlacement?.profiles.find((profile) => profile.id === effectiveProfileId) ?? null
    const enabledProfiles = executionPlacement?.profiles.filter((profile) => profile.enabled && profile.id) ?? []
    const validationMessage = (executionPlacement?.validation_errors.length ? 'Execution profile settings are invalid.' : null)
        || (effectiveProfileId && executionPlacement && !effectiveProfile
            ? `Execution profile ${effectiveProfileId} is not available.`
            : null)
        || (effectiveProfile && !effectiveProfile.enabled
            ? `Execution profile ${effectiveProfileId} is disabled.`
            : null)
    const statusMessage = workspaceSettingsError || validationMessage

    return {
        selectedProfileId,
        setSelectedProfileId,
        effectiveProfileId,
        effectiveProfile,
        enabledProfiles,
        projectDefaultProfileId,
        validationMessage,
        statusMessage,
    }
}
