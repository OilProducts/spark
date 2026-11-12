import { ChevronDown, ChevronUp } from 'lucide-react'

import type { LaunchInputDefinition, LaunchInputFormValues, ParsedLaunchInputDefinitions } from '@/lib/flowContracts'
import { InlineError } from '@/components/app/inline-error'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { NativeSelect } from '@/components/ui/native-select'
import { Textarea } from '@/components/ui/textarea'

interface LaunchInputsFormProps {
    flowName: string | null
    parsedLaunchInputs: ParsedLaunchInputDefinitions
    launchInputValues: LaunchInputFormValues
    launchInputsCollapsed: boolean
    onToggleCollapsed: () => void
    onInputChange: (entry: LaunchInputDefinition, value: string) => void
}

/** One field per row, label and description above the control, so the panel's width never squeezes them. */
export function LaunchInputsForm({
    flowName,
    parsedLaunchInputs,
    launchInputValues,
    launchInputsCollapsed,
    onToggleCollapsed,
    onInputChange,
}: LaunchInputsFormProps) {
    const canCollapseLaunchInputs = parsedLaunchInputs.entries.length > 0
    return (
        <div
            data-testid="execution-launch-inputs"
            className="w-full"
        >
            <div
                data-testid="execution-launch-inputs-toolbar"
                className="mb-2 flex items-center justify-between gap-3"
            >
                <p
                    data-testid="execution-launch-inputs-title"
                    className="text-sm font-semibold text-foreground"
                >
                    What this run needs
                </p>
                {canCollapseLaunchInputs ? (
                    <Button
                        type="button"
                        data-testid="execution-launch-inputs-toggle"
                        aria-label={launchInputsCollapsed ? 'Show run inputs' : 'Hide run inputs'}
                        title={launchInputsCollapsed ? 'Show run inputs' : 'Hide run inputs'}
                        aria-expanded={!launchInputsCollapsed}
                        onClick={onToggleCollapsed}
                        variant="ghost"
                        size="icon-xs"
                        className="h-7 w-7 text-muted-foreground hover:bg-muted/60 hover:text-foreground"
                    >
                        {launchInputsCollapsed ? (
                            <ChevronDown className="h-3.5 w-3.5" />
                        ) : (
                            <ChevronUp className="h-3.5 w-3.5" />
                        )}
                    </Button>
                ) : null}
            </div>
            {parsedLaunchInputs.error ? (
                <InlineError data-testid="execution-launch-inputs-schema-error" className="mb-3" dense>{parsedLaunchInputs.error}</InlineError>
            ) : null}
            {!launchInputsCollapsed ? (
                <div
                    data-testid="execution-launch-inputs-grid"
                    className="space-y-4"
                >
                    {parsedLaunchInputs.entries.map((entry) => {
                        const id = `launch-input-${flowName || 'flow'}-${entry.key}`.replace(/[^\w-]/g, '-')
                        const descriptionId = entry.description ? `${id}-description` : undefined
                        const common = {
                            id,
                            'data-testid': `execution-launch-input-${entry.key}`,
                            'aria-describedby': descriptionId,
                            'aria-required': entry.required || undefined,
                            value: launchInputValues[entry.key] ?? '',
                        }
                        return (
                            <div
                                key={`${flowName || 'flow'}-${entry.key}`}
                                data-testid={`execution-launch-input-field-${entry.key}`}
                                className="space-y-1.5"
                            >
                                <div>
                                    <Label htmlFor={id} className="text-sm">
                                        {entry.label}
                                        {entry.required ? null : <span className="font-normal text-muted-foreground">(optional)</span>}
                                    </Label>
                                    {entry.description ? (
                                        <p id={descriptionId} className="mt-0.5 text-xs leading-5 text-muted-foreground">
                                            {entry.description}
                                        </p>
                                    ) : null}
                                </div>
                                {entry.type === 'string' ? (
                                    <Input
                                        {...common}
                                        onChange={(event) => onInputChange(entry, event.target.value)}
                                        className="h-8 text-sm"
                                    />
                                ) : entry.type === 'string[]' ? (
                                    <Textarea
                                        {...common}
                                        onChange={(event) => onInputChange(entry, event.target.value)}
                                        rows={2}
                                        className="min-h-16 px-2 py-1 text-sm"
                                        placeholder="One item per line"
                                    />
                                ) : entry.type === 'boolean' ? (
                                    <NativeSelect
                                        {...common}
                                        onChange={(event) => onInputChange(entry, event.target.value)}
                                        size="sm"
                                        className="w-40 text-sm"
                                    >
                                        <option value="">Not set</option>
                                        <option value="true">Yes</option>
                                        <option value="false">No</option>
                                    </NativeSelect>
                                ) : entry.type === 'number' ? (
                                    <Input
                                        {...common}
                                        type="number"
                                        onChange={(event) => onInputChange(entry, event.target.value)}
                                        className="h-8 w-40 text-sm"
                                    />
                                ) : (
                                    <Textarea
                                        {...common}
                                        onChange={(event) => onInputChange(entry, event.target.value)}
                                        rows={3}
                                        className="min-h-20 px-2 py-1 font-mono text-sm"
                                        placeholder='{"key": "value"}'
                                    />
                                )}
                            </div>
                        )
                    })}
                </div>
            ) : null}
        </div>
    )
}
