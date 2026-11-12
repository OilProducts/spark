import { Label } from '@/components/ui/label'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'

import type { ExecutionPlacementState } from '../hooks/useExecutionPlacement'

interface ExecutionProfileSectionProps {
    placement: ExecutionPlacementState
}

/** Where the run executes: the project's default profile unless this run picks another. */
export function ExecutionProfileSection({ placement }: ExecutionProfileSectionProps) {
    const {
        selectedProfileId,
        setSelectedProfileId,
        defaultProfileLabel,
        enabledProfiles,
        validationMessage,
        statusMessage,
    } = placement
    return (
        <div
            data-testid="execution-profile-launch-settings"
            className="space-y-1.5 [&>[data-slot=native-select-wrapper]]:w-full"
        >
            <Label htmlFor="execution-profile-override" className="text-sm">
                Runs on
            </Label>
            <NativeSelect
                id="execution-profile-override"
                data-testid="execution-profile-override-select"
                value={selectedProfileId}
                onChange={(event) => setSelectedProfileId(event.target.value)}
                aria-describedby={statusMessage ? 'execution-profile-status' : undefined}
                aria-invalid={validationMessage ? true : undefined}
                size="sm"
                className="w-full text-sm"
            >
                <NativeSelectOption value="">
                    Project default ({defaultProfileLabel})
                </NativeSelectOption>
                {enabledProfiles.map((profile) => (
                    <NativeSelectOption key={profile.id} value={profile.id || ''}>
                        {profile.label || profile.id}
                    </NativeSelectOption>
                ))}
            </NativeSelect>
            {statusMessage ? (
                <p
                    id="execution-profile-status"
                    data-testid="execution-profile-prelaunch-validation"
                    className={`text-xs ${validationMessage ? 'text-destructive' : 'text-muted-foreground'}`}
                >
                    {statusMessage}
                </p>
            ) : null}
        </div>
    )
}
