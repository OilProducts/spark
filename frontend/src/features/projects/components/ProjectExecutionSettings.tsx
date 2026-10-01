import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader } from '@/components/ui/card'
import { Label } from '@/components/ui/label'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import type { ExecutionPlacementProfile } from '@/lib/workspaceClient'
import { useProjectExecutionSettings, WORKSPACE_DEFAULT_VALUE } from '../hooks/useProjectExecutionSettings'

function formatProfileOption(profile: ExecutionPlacementProfile): string {
    const profileId = profile.id ?? ''
    const label = profile.label?.trim()
    if (label && label !== profileId) {
        return `${label} (${profileId})`
    }
    return profileId
}

export function ProjectExecutionSettings({ projectPath }: { projectPath: string }) {
    const {
        dirty,
        message,
        discard,
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
    } = useProjectExecutionSettings(projectPath)

    return (
        <Card data-testid="project-settings-dialog" className="gap-4 py-4">
            <CardHeader className="px-4"><h3 className="text-lg font-light">Execution profile</h3></CardHeader>
            <CardContent className="space-y-4 px-4">
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
                <div className="flex justify-end gap-2">
                    <Button aria-label="Discard project settings changes" type="button" variant="outline" size="sm" disabled={!dirty || isSaving || isLoading} onClick={discard}>
                        Discard
                    </Button>
                    <Button
                        type="button"
                        data-testid="project-settings-save-button"
                        aria-label={isSaving ? undefined : 'Save project settings'}
                        size="sm"
                        disabled={!canSave || !dirty}
                        onClick={() => {
                            void onSave()
                        }}
                    >
                        {isSaving ? 'Saving...' : 'Save'}
                    </Button>
                </div>
            </CardContent>
        </Card>
    )
}
