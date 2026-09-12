import { useEffect, useMemo, useState } from 'react'
import { useDialogController } from '@/components/app/dialog-controller'
import { useSettingsNavigationProtection } from '@/features/settings/hooks/useSettingsNavigationProtection'

import {
    ApiHttpError,
    ApiSchemaError,
    fetchWorkspaceSettingsValidated,
    updateProjectStateValidated,
    type WorkspaceSettingsResponse,
} from '@/lib/workspaceClient'
import { useStore } from '@/store'
import { fetchProjectExecutionSettings } from '@/lib/api/settingsApi'
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
) {
    const upsertProjectRegistryEntry = useStore((state) => state.upsertProjectRegistryEntry)
    const [revision, setRevision] = useState<string | null>(null)
    const [settings, setSettings] = useState<WorkspaceSettingsResponse | null>(null)
    const [loadError, setLoadError] = useState<string | null>(null)
    const [saveError, setSaveError] = useState<string | null>(null)
    const [isLoading, setLoading] = useState(false)
    const [isSaving, setSaving] = useState(false)
    const [selectedProfileValue, setSelectedProfileValue] = useState(WORKSPACE_DEFAULT_VALUE)

    const [savedProfileValue, setSavedProfileValue] = useState(WORKSPACE_DEFAULT_VALUE)
    const [message, setMessage] = useState('')
    const [reload, setReload] = useState(0)
    const { confirm } = useDialogController()
    const dirty = open && revision !== null && selectedProfileValue !== savedProfileValue
    useSettingsNavigationProtection(dirty, isSaving)
    const requestOpenChange = async (next: boolean) => {
        if (isSaving) return
        if (!next && dirty && !await confirm({ title: 'Discard unsaved settings?', description: 'Your unsaved project settings will be lost.', confirmLabel: 'Discard and leave', cancelLabel: 'Keep editing' })) return
        onOpenChange(next)
    }
    const discard = () => {
        if (isSaving) return
        setRevision(null)
        setSelectedProfileValue(WORKSPACE_DEFAULT_VALUE)
        setSavedProfileValue(WORKSPACE_DEFAULT_VALUE)
        setMessage('')
        setSaveError(null)
        setReload((value) => value + 1)
    }
    useEffect(() => {
        setRevision(null)
        setSelectedProfileValue(WORKSPACE_DEFAULT_VALUE)
        setSavedProfileValue(WORKSPACE_DEFAULT_VALUE)
        setSettings(null)
        setMessage('')
        setSaveError(null)
    }, [open, projectPath])

    useEffect(() => {
        if (!open || !projectPath) {
            return
        }
        let cancelled = false
        const refresh = () => {
            if (isSaving) return
            if (!revision) setLoading(true)
            void Promise.all([fetchWorkspaceSettingsValidated(), fetchProjectExecutionSettings(projectPath)])
            .then(([response, execution]) => {
                if (cancelled) return
                if (dirty) {
                    if (execution.revision !== revision || JSON.stringify(response.execution_placement) !== JSON.stringify(settings?.execution_placement)) {
                        setMessage('Project execution settings changed elsewhere. Your draft is retained; Discard reloads the latest values.')
                    }
                    return
                }
                setLoadError(null)
                setMessage('')
                setSettings((previous) => JSON.stringify(previous) === JSON.stringify(response) ? previous : response)
                setSavedProfileValue(execution.stored || WORKSPACE_DEFAULT_VALUE)
                setRevision(execution.revision)
                setSelectedProfileValue(execution.stored || WORKSPACE_DEFAULT_VALUE)
            })
            .catch((error) => {
                if (cancelled) return
                const fallback = error instanceof ApiSchemaError
                    ? error.message
                    : 'Unable to load workspace execution profiles.'
                const message = error instanceof ApiHttpError && error.detail
                    ? error.detail
                    : extractApiErrorMessage(error, fallback)
                setLoadError(message)
            })
            .finally(() => {
                if (!cancelled) setLoading(false)
            })
        }
        refresh()
        window.addEventListener('spark:settings-live-event', refresh)
        window.addEventListener('focus', refresh)
        return () => {
            cancelled = true
            window.removeEventListener('spark:settings-live-event', refresh)
            window.removeEventListener('focus', refresh)
        }
    }, [open, projectPath, dirty, isSaving, revision, settings, reload])

    const enabledProfiles = useMemo(
        () => (settings?.execution_placement.profiles ?? []).filter((profile) => profile.enabled && profile.id),
        [settings],
    )
    const settingsError = buildSettingsError(settings, loadError)
    const canSave = Boolean(projectPath) && Boolean(revision) && !isLoading && !isSaving && !settingsError

    const onSave = async () => {
        if (!projectPath || !revision || !canSave) {
            return
        }
        setSaving(true)
        setSaveError(null)
        try {
            const projectRecord = await updateProjectStateValidated({
                project_path: projectPath,
                expected_revision: revision,
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
        dirty,
        message,
        discard,
        requestOpenChange,
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
