import { Badge } from '@/components/ui/badge'
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
    const profileName = effectiveProfile
        ? `${effectiveProfile.label || effectiveProfile.id} (${effectiveProfile.mode})`
        : effectiveProfileId
    return (
        <div
            data-testid="execution-profile-launch-settings"
            className="space-y-2 rounded-lg border border-border/80 bg-muted/10 p-4"
        >
            <div className="flex min-w-0 items-baseline justify-between gap-3">
                <p className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
                    Execution profile
                </p>
                <p
                    data-testid="execution-profile-effective-id"
                    className="min-w-0 truncate font-mono text-xs text-muted-foreground"
                    title={effectiveProfileId}
                >
                    {effectiveProfileId}
                </p>
            </div>
            <div className="flex min-w-0 items-center gap-3">
                <p
                    data-testid="execution-profile-effective-selection"
                    className="min-w-0 truncate text-sm text-foreground"
                    title={profileName}
                >
                    {profileName}
                </p>
                {statusMessage ? null : (
                    <Badge
                        data-testid="execution-profile-prelaunch-validation"
                        variant="outline"
                        className="text-muted-foreground"
                    >
                        Ready
                    </Badge>
                )}
                <div className="min-w-32 flex-1 [&>[data-slot=native-select-wrapper]]:w-full">
                    <NativeSelect
                        id="execution-profile-override"
                        data-testid="execution-profile-override-select"
                        aria-label="Run override"
                        value={selectedProfileId}
                        onChange={(event) => setSelectedProfileId(event.target.value)}
                        size="sm"
                        className="w-full text-sm"
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
            {statusMessage ? (
                <p
                    data-testid="execution-profile-prelaunch-validation"
                    className={`text-xs ${validationMessage ? 'text-destructive' : 'text-muted-foreground'}`}
                >
                    {statusMessage}
                </p>
            ) : null}
        </div>
    )
}
