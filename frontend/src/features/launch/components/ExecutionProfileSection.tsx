import { Label } from '@/components/ui/label'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'

import type { ExecutionPlacementState } from '../hooks/useExecutionPlacement'

interface ExecutionProfileSectionProps {
    placement: ExecutionPlacementState
}

export function ExecutionProfileSection({ placement }: ExecutionProfileSectionProps) {
    const {
        selectedProfileId,
        setSelectedProfileId,
        effectiveProfileId,
        effectiveProfile,
        enabledProfiles,
        validationMessage,
        statusMessage,
    } = placement
    return (
        <div
            data-testid="execution-profile-launch-settings"
            className="grid gap-3 rounded-lg border border-border/80 bg-muted/10 p-4 md:grid-cols-[minmax(0,1fr)_minmax(12rem,16rem)] md:items-end"
        >
            <div className="min-w-0 space-y-1">
                <p className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
                    Execution profile
                </p>
                <p
                    data-testid="execution-profile-effective-selection"
                    className="truncate text-sm text-foreground"
                    title={effectiveProfileId}
                >
                    {effectiveProfile
                        ? `${effectiveProfile.label || effectiveProfile.id} (${effectiveProfile.mode})`
                        : effectiveProfileId}
                </p>
                <p
                    data-testid="execution-profile-effective-id"
                    className="font-mono text-xs text-muted-foreground"
                >
                    {effectiveProfileId}
                </p>
                {statusMessage ? (
                    <p
                        data-testid="execution-profile-prelaunch-validation"
                        className={`text-xs ${validationMessage ? 'text-destructive' : 'text-muted-foreground'}`}
                    >
                        {statusMessage}
                    </p>
                ) : (
                    <p
                        data-testid="execution-profile-prelaunch-validation"
                        className="text-xs text-muted-foreground"
                    >
                        Ready
                    </p>
                )}
            </div>
            <div className="space-y-1.5">
                <Label htmlFor="execution-profile-override" className="text-xs">
                    Run override
                </Label>
                <NativeSelect
                    id="execution-profile-override"
                    data-testid="execution-profile-override-select"
                    value={selectedProfileId}
                    onChange={(event) => setSelectedProfileId(event.target.value)}
                    size="sm"
                    className="w-full text-xs"
                >
                    <NativeSelectOption value="">
                        Project/runtime default
                    </NativeSelectOption>
                    {enabledProfiles.map((profile) => (
                        <NativeSelectOption key={profile.id} value={profile.id || ''}>
                            {profile.label || profile.id}
                        </NativeSelectOption>
                    ))}
                </NativeSelect>
            </div>
        </div>
    )
}
