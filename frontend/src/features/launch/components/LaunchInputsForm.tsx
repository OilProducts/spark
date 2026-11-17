import { ChevronDown, ChevronUp } from 'lucide-react'

import type { LaunchInputDefinition, LaunchInputFormValues, ParsedLaunchInputDefinitions } from '@/lib/flowContracts'
import { InlineError } from '@/components/app/inline-error'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { NativeSelect } from '@/components/ui/native-select'
import { Textarea } from '@/components/ui/textarea'

interface LaunchInputsFormProps {
    /** The section's heading. */
    title?: string
    flowName: string | null
    parsedLaunchInputs: ParsedLaunchInputDefinitions
    launchInputValues: LaunchInputFormValues
    /** What is wrong with each input, by key, after a run was asked for. */
    errors?: Record<string, string>
    launchInputsCollapsed: boolean
    onToggleCollapsed: () => void
    onInputChange: (entry: LaunchInputDefinition, value: string) => void
}

/** One field per row, label and description above the control, so the panel's width never squeezes them. */
export function LaunchInputsForm({
    title = 'What this run needs',
    flowName,
    parsedLaunchInputs,
    launchInputValues,
    errors = {},
    launchInputsCollapsed,
    onToggleCollapsed,
    onInputChange,
}: LaunchInputsFormProps) {
    const canCollapseLaunchInputs = parsedLaunchInputs.entries.length > 0
    const hasRequired = parsedLaunchInputs.entries.some((entry) => entry.required)
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
                    {title}
                    {hasRequired ? <span className="ml-2 text-xs font-normal text-muted-foreground"><span aria-hidden="true" className="text-destructive">*</span> required</span> : null}
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
                        const error = errors[entry.key]
                        const errorId = error ? `${id}-error` : undefined
                        const common = {
                            id,
                            'data-testid': `execution-launch-input-${entry.key}`,
                            'aria-describedby': [errorId, descriptionId].filter(Boolean).join(' ') || undefined,
                            'aria-required': entry.required || undefined,
                            'aria-invalid': error ? true : undefined,
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
                                        {entry.required ? <span aria-hidden="true" className="text-destructive">*</span> : <span className="font-normal text-muted-foreground">(optional)</span>}
                                    </Label>
                                    {entry.description ? (
                                        <p id={descriptionId} className="mt-0.5 text-xs leading-5 text-muted-foreground">
                                            {entry.description}
                                        </p>
                                    ) : null}
                                </div>
                                {entry.type === 'string' ? (
                                    // Text may be a sentence or a bug report: one row that grows with its lines.
                                    <Textarea
                                        {...common}
                                        onChange={(event) => onInputChange(entry, event.target.value)}
                                        rows={1}
                                        className="max-h-72 min-h-8 px-2 py-1.5 text-sm"
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
                                {error ? (
                                    <p id={errorId} data-testid={`execution-launch-input-error-${entry.key}`} className="text-xs text-destructive">{error}</p>
                                ) : null}
                            </div>
                        )
                    })}
                </div>
            ) : null}
        </div>
    )
}
