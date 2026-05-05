import { useEffect, useMemo, useState } from 'react'

import {
    Dialog,
    DialogContent,
    DialogDescription,
    DialogFooter,
    DialogHeader,
    DialogTitle,
} from '@/components/ui/dialog'
import { Button } from '@/components/ui/button'
import { Label } from '@/components/ui/label'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
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

const WORKSPACE_DEFAULT_VALUE = '__workspace_default__'

function formatProfileOption(profile: ExecutionPlacementProfile): string {
    const profileId = profile.id ?? ''
    const label = profile.label?.trim()
    if (label && label !== profileId) {
        return `${label} (${profileId})`
    }
    return profileId
}

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

type ProjectSettingsDialogProps = {
    open: boolean
    projectPath: string | null
    onOpenChange: (open: boolean) => void
}

export function ProjectSettingsDialog({
    open,
    projectPath,
    onOpenChange,
}: ProjectSettingsDialogProps) {
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

    return (
        <Dialog open={open} onOpenChange={onOpenChange}>
            <DialogContent data-testid="project-settings-dialog" className="sm:max-w-md">
                <DialogHeader>
                    <DialogTitle data-testid="project-settings-title">
                        {projectPath || 'No active project'}
                    </DialogTitle>
                    <DialogDescription>
                        Project settings
                    </DialogDescription>
                </DialogHeader>
                <div className="space-y-4">
                    <div className="space-y-2">
                        <Label htmlFor="project-default-execution-profile">Default execution profile</Label>
                        <Select
                            value={selectedProfileValue}
                            onValueChange={setSelectedProfileValue}
                            disabled={isLoading || Boolean(settingsError)}
                        >
                            <SelectTrigger
                                id="project-default-execution-profile"
                                data-testid="project-default-execution-profile"
                                className="w-full"
                                aria-invalid={Boolean(settingsError)}
                            >
                                <SelectValue placeholder="Use workspace default" />
                            </SelectTrigger>
                            <SelectContent>
                                <SelectItem value={WORKSPACE_DEFAULT_VALUE}>Use workspace default</SelectItem>
                                {enabledProfiles.map((profile) => (
                                    <SelectItem key={profile.id} value={profile.id ?? ''}>
                                        {formatProfileOption(profile)}
                                    </SelectItem>
                                ))}
                            </SelectContent>
                        </Select>
                    </div>
                    {isLoading ? (
                        <p data-testid="project-settings-loading" className="text-xs text-muted-foreground">
                            Loading execution profiles...
                        </p>
                    ) : null}
                    {settingsError ? (
                        <p data-testid="project-settings-error" className="text-xs text-destructive">
                            {settingsError}
                        </p>
                    ) : null}
                    {saveError ? (
                        <p data-testid="project-settings-save-error" className="text-xs text-destructive">
                            {saveError}
                        </p>
                    ) : null}
                </div>
                <DialogFooter>
                    <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
                        Cancel
                    </Button>
                    <Button
                        type="button"
                        data-testid="project-settings-save-button"
                        disabled={!canSave}
                        onClick={() => {
                            void onSave()
                        }}
                    >
                        {isSaving ? 'Saving...' : 'Save'}
                    </Button>
                </DialogFooter>
            </DialogContent>
        </Dialog>
    )
}
