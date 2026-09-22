import {
    Dialog,
    DialogContent,
    DialogDescription,
    DialogFooter,
    DialogHeader,
    DialogTitle,
} from '@/components/ui/dialog'
import { Trash2, X } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Label } from '@/components/ui/label'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import type { ExecutionPlacementProfile } from '@/lib/workspaceClient'
import { useProjectSettingsDialog, WORKSPACE_DEFAULT_VALUE } from './useProjectSettingsDialog'

function formatProfileOption(profile: ExecutionPlacementProfile): string {
    const profileId = profile.id ?? ''
    const label = profile.label?.trim()
    if (label && label !== profileId) {
        return `${label} (${profileId})`
    }
    return profileId
}

type ProjectSettingsDialogProps = {
    open: boolean
    projectPath: string | null
    onOpenChange: (open: boolean) => void
    onClearProject: () => void
    onRemoveProject: () => Promise<void>
}

export function ProjectSettingsDialog({
    open,
    projectPath,
    onOpenChange,
    onClearProject,
    onRemoveProject,
}: ProjectSettingsDialogProps) {
    const {
        dirty,
        message,
        discard,
        requestOpenChange,
        enabledProfiles,
        settingsError,
        validationError,
        saveError,
        isLoading,
        isSaving,
        canSave,
        selectedProfileValue,
        setSelectedProfileValue,
        onSave,
    } = useProjectSettingsDialog(open, projectPath, onOpenChange)

    return (
        <Dialog open={open} onOpenChange={(next) => void requestOpenChange(next)}>
            <DialogContent data-testid="project-settings-dialog" className="sm:max-w-md">
                <DialogHeader>
                    <DialogTitle data-testid="project-settings-title">
                        {projectPath || 'No active project'}
                    </DialogTitle>
                    <DialogDescription>
                        Set this project's execution defaults, or clear or remove it from Spark.
                    </DialogDescription>
                </DialogHeader>
                <div className="space-y-4">
                    <div className="space-y-2">
                        <Label htmlFor="project-default-execution-profile">Default execution profile</Label>
                        <Select
                            value={selectedProfileValue}
                            onValueChange={setSelectedProfileValue}
                            disabled={isLoading || isSaving || Boolean(settingsError)}
                        >
                            <SelectTrigger
                                id="project-default-execution-profile"
                                data-testid="project-default-execution-profile"
                                className="w-full"
                                aria-invalid={Boolean(settingsError || validationError)}
                            >
                                <SelectValue placeholder={validationError ? "Select a replacement or workspace default" : "Use workspace default"} />
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
                    {settingsError || validationError ? (
                        <p data-testid="project-settings-error" className="text-xs text-destructive">
                            {settingsError || validationError}
                        </p>
                    ) : null}
                    {message && <p role="status" className="text-xs">{message}</p>}
                    {saveError ? (
                        <p data-testid="project-settings-save-error" className="text-xs text-destructive">
                            {saveError}
                        </p>
                    ) : null}
                    <div className="space-y-2 border-t pt-4">
                        <p className="text-sm font-medium">Project</p>
                        <p className="text-xs text-muted-foreground">
                            Clearing only deselects the project. Removing deletes its Spark threads, workflow history, and runs; project files stay on disk.
                        </p>
                        {/* Project changes are navigation-guarded; disabled while dirty so a discard prompt cannot race this dialog closing. */}
                        <div className="flex flex-wrap gap-2">
                            <Button
                                type="button"
                                data-testid="top-nav-project-clear-button"
                                variant="outline"
                                size="sm"
                                disabled={!projectPath || dirty || isSaving}
                                onClick={() => {
                                    onClearProject()
                                    onOpenChange(false)
                                }}
                            >
                                <X className="h-3.5 w-3.5" />
                                Clear active project
                            </Button>
                            <Button
                                type="button"
                                data-testid="top-nav-project-remove-button"
                                variant="outline"
                                size="sm"
                                disabled={!projectPath || dirty || isSaving}
                                className="border-destructive/40 text-destructive hover:bg-destructive/10"
                                onClick={() => {
                                    void onRemoveProject().then(() => onOpenChange(false))
                                }}
                            >
                                <Trash2 className="h-3.5 w-3.5" />
                                Remove project…
                            </Button>
                        </div>
                    </div>
                </div>
                <DialogFooter>
                    <Button type="button" variant="outline" disabled={isSaving || isLoading} onClick={discard}>
                        Discard
                    </Button>
                    <Button type="button" variant="outline" disabled={isSaving} onClick={() => void requestOpenChange(false)}>
                        Cancel
                    </Button>
                    <Button
                        type="button"
                        data-testid="project-settings-save-button"
                        disabled={!canSave || !dirty}
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
