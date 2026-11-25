import { useEffect, useState } from 'react'

import {
    fetchWorkspaceSettingsValidated,
    type ExecutionPlacementProfile,
    type WorkspaceSettingsResponse,
} from '@/lib/workspaceClient'
import { normalizeProjectPath } from '@/lib/projectPaths'
import { useStore } from '@/store'

/** Where a profile's mode runs work, in plain words. */
export const EXECUTION_MODE_PLACES: Record<ExecutionPlacementProfile['mode'], string> = {
    native: 'on this computer',
    local_container: 'in a container',
}

const profileLabel = (profile: ExecutionPlacementProfile) => profile.label || profile.id || ''

/** "on this computer", or "in a container: GPU box" when the label says more than the mode. */
function describeProfile(profile: ExecutionPlacementProfile): string {
    const place = EXECUTION_MODE_PLACES[profile.mode]
    const label = profileLabel(profile)
    const normalized = label.trim().toLowerCase().replace(/[\s_-]+/g, ' ')
    const isModeName = !normalized || normalized === profile.mode.replace('_', ' ') || (profile.mode === 'local_container' && normalized === 'container')
    return isModeName ? place : `${place}: ${label}`
}

export interface ExecutionPlacementState {
    selectedProfileId: string
    setSelectedProfileId: (profileId: string) => void
    effectiveProfileId: string
    effectiveProfile: ExecutionPlacementProfile | null
    /** The profile a run uses when none is picked: the project's, else the workspace's. */
    defaultProfileId: string
    /** What the "default" choice means here: the project's profile, else the runtime's. */
    defaultProfileLabel: string
    /**
     * Where the run will execute, in plain lowercase words for a " · "-joined summary:
     * "on this computer", or "in a container: <label>" when the label says more than the mode.
     */
    effectiveProfileSummary: string
    enabledProfiles: ExecutionPlacementProfile[]
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
        ? projectRegistry[normalizeProjectPath(projectPath)]?.executionProfileId || null
        : null
    const defaultProfileId = projectDefaultProfileId
        || executionPlacement?.default_execution_profile_id
        || 'native'
    const effectiveProfileId = selectedProfileId || defaultProfileId
    const defaultProfile = executionPlacement?.profiles.find((profile) => profile.id === defaultProfileId)
    const defaultProfileLabel = defaultProfile?.label || defaultProfileId
    const effectiveProfile = executionPlacement?.profiles.find((profile) => profile.id === effectiveProfileId) ?? null
    const enabledProfiles = executionPlacement?.profiles.filter((profile) => profile.enabled && profile.id) ?? []
    const validationMessage = (executionPlacement?.validation_errors.length
        ? 'The execution profiles in Settings have errors. Fix them to run.'
        : null)
        || (effectiveProfileId && executionPlacement && !effectiveProfile
            ? `This run's execution profile, ${effectiveProfileId}, isn't available.`
            : null)
        || (effectiveProfile && !effectiveProfile.enabled
            ? `This run's execution profile, ${profileLabel(effectiveProfile)}, is turned off in Settings.`
            : null)
    const statusMessage = workspaceSettingsError || validationMessage

    return {
        selectedProfileId,
        setSelectedProfileId,
        effectiveProfileId,
        effectiveProfile,
        defaultProfileId,
        defaultProfileLabel,
        effectiveProfileSummary: effectiveProfile ? describeProfile(effectiveProfile) : effectiveProfileId,
        enabledProfiles,
        validationMessage,
        statusMessage,
    }
}
