import { useEffect, useMemo, useState } from 'react'

import {
    ApiHttpError,
    ApiSchemaError,
    fetchWorkspaceSettingsValidated,
    updateProjectStateValidated,
    type ExecutionPlacementProfile,
    type WorkspaceSettingsResponse,
} from '@/lib/workspaceClient'
import { useStore } from '@/store'
import { extractApiErrorMessage, toHydratedProjectRecord } from '@/features/projects/model/projectsHomeState'

export const WORKSPACE_DEFAULT_VALUE = '__workspace_default__'

function buildSettingsError(settings: WorkspaceSettingsResponse | null, loadError: string | null): string | null {
    if (loadError) {
        return loadError
    }
    const placement = settings?.execution_placement
    if (!placement) {
        return null
    }
    if (!placement.config.loaded) {
        return `Execution profile config could not be loaded from ${placement.config.path}.`
    }
    if (placement.validation_errors.length > 0) {
        return placement.validation_errors.map((error) => error.message).join(' ')
    }
    return null
}

export function useProjectSettingsDialog(
    open: boolean,
    projectPath: string | null,
    onOpenChange: (open: boolean) => void,
): {
    enabledProfiles: ExecutionPlacementProfile[]
    settingsError: string | null
    saveError: string | null
    isLoading: boolean
    isSaving: boolean
    canSave: boolean
    selectedProfileValue: string
    setSelectedProfileValue: (value: string) => void
    onSave: () => Promise<void>
} {
    const project = useStore((state) => (projectPath ? state.projectRegistry[projectPath] : null))
    const upsertProjectRegistryEntry = useStore((state) => state.upsertProjectRegistryEntry)
    const [settings, setSettings] = useState<WorkspaceSettingsResponse | null>(null)
    const [loadError, setLoadError] = useState<string | null>(null)
    const [saveError, setSaveError] = useState<string | null>(null)
    const [isLoading, setLoading] = useState(false)
    const [isSaving, setSaving] = useState(false)
    const [selectedProfileValue, setSelectedProfileValue] = useState(WORKSPACE_DEFAULT_VALUE)

    useEffect(() => {
        if (!open) {
            return
        }
        setSelectedProfileValue(project?.executionProfileId || WORKSPACE_DEFAULT_VALUE)
        setSettings(null)
        setLoadError(null)
        setSaveError(null)
        setLoading(true)
        fetchWorkspaceSettingsValidated()
            .then((response) => {
                setSettings(response)
            })
            .catch((error) => {
                const fallback = error instanceof ApiSchemaError
                    ? error.message
                    : 'Unable to load workspace execution profiles.'
                const message = error instanceof ApiHttpError && error.detail
                    ? error.detail
                    : extractApiErrorMessage(error, fallback)
                setLoadError(message)
            })
            .finally(() => {
                setLoading(false)
            })
    }, [open, project?.executionProfileId])

    const enabledProfiles = useMemo(
        () => (settings?.execution_placement.profiles ?? []).filter((profile) => profile.enabled && profile.id),
        [settings],
    )
    const settingsError = buildSettingsError(settings, loadError)
    const canSave = Boolean(projectPath) && !isLoading && !isSaving && !settingsError

    const onSave = async () => {
        if (!projectPath || !canSave) {
            return
        }
        setSaving(true)
        setSaveError(null)
        try {
            const projectRecord = await updateProjectStateValidated({
                project_path: projectPath,
                execution_profile_id: selectedProfileValue === WORKSPACE_DEFAULT_VALUE ? null : selectedProfileValue,
            })
            upsertProjectRegistryEntry(toHydratedProjectRecord(projectRecord))
            onOpenChange(false)
        } catch (error) {
            setSaveError(extractApiErrorMessage(error, 'Unable to save project settings.'))
        } finally {
            setSaving(false)
        }
    }

    return {
        enabledProfiles,
        settingsError,
        saveError,
        isLoading,
        isSaving,
        canSave,
        selectedProfileValue,
        setSelectedProfileValue,
        onSave,
    }
}
