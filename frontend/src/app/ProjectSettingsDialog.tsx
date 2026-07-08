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
import type { ExecutionPlacementProfile } from '@/lib/workspaceClient'
import { useProjectSettingsForm, WORKSPACE_DEFAULT_VALUE } from './useProjectSettingsForm'

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
}

export function ProjectSettingsDialog({
    open,
    projectPath,
    onOpenChange,
}: ProjectSettingsDialogProps) {
    const {
        selectedProfileValue,
        setSelectedProfileValue,
        enabledProfiles,
        settingsError,
        saveError,
        isLoading,
        isSaving,
        canSave,
        onSave,
    } = useProjectSettingsForm({ open, projectPath, onOpenChange })

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
