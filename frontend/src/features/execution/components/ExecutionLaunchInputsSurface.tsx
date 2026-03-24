import { ChevronDown, ChevronUp } from 'lucide-react'

import type { LaunchInputDefinition, LaunchInputFormValues, ParsedLaunchInputDefinitions } from '@/lib/flowContracts'

const launchInputTypeLabel = (type: string) => {
    switch (type) {
        case 'string':
            return 'Text'
        case 'string[]':
            return 'List'
        case 'number':
            return 'Number'
        case 'boolean':
            return 'Boolean'
        default:
            return 'JSON'
    }
}

const launchInputDesktopSpanClass = (
    type: string,
    index: number,
    totalEntries: number,
    required: boolean,
) => {
    if (totalEntries === 1) {
        return 'lg:col-span-12'
    }
    if (type === 'boolean' || type === 'number') {
        return 'lg:col-span-4'
    }
    if (index === 0 && required && type === 'string' && totalEntries <= 3) {
        return 'lg:col-span-12'
    }
    return 'lg:col-span-6'
}

interface ExecutionLaunchInputsSurfaceProps {
    isNarrowViewport: boolean
    executionFlowName: string | null
    parsedLaunchInputs: ParsedLaunchInputDefinitions
    launchInputValues: LaunchInputFormValues
    launchInputCount: number
    launchInputsCollapsed: boolean
    canCollapseLaunchInputs: boolean
    onToggleCollapsed: () => void
    onInputChange: (entry: LaunchInputDefinition, value: string) => void
}

export function ExecutionLaunchInputsSurface({
    isNarrowViewport,
    executionFlowName,
    parsedLaunchInputs,
    launchInputValues,
    launchInputCount,
    launchInputsCollapsed,
    canCollapseLaunchInputs,
    onToggleCollapsed,
    onInputChange,
}: ExecutionLaunchInputsSurfaceProps) {
    return (
        <div
            data-testid="execution-launch-inputs"
            className="mb-3 w-full"
        >
            <div
                data-testid="execution-launch-inputs-toolbar"
                className="mb-2 flex items-center justify-between gap-3"
            >
                <p
                    data-testid="execution-launch-inputs-title"
                    className="text-[11px] font-semibold uppercase tracking-wide text-muted-foreground"
                >
                    Launch Inputs
                </p>
                {canCollapseLaunchInputs ? (
                    <button
                        type="button"
                        data-testid="execution-launch-inputs-toggle"
                        aria-label={launchInputsCollapsed ? 'Expand launch inputs' : 'Collapse launch inputs'}
                        title={launchInputsCollapsed ? 'Expand launch inputs' : 'Collapse launch inputs'}
                        onClick={onToggleCollapsed}
                        className="inline-flex h-7 w-7 items-center justify-center rounded-md text-muted-foreground hover:bg-muted/60 hover:text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                    >
                        {launchInputsCollapsed ? (
                            <ChevronUp className="h-3.5 w-3.5" />
                        ) : (
                            <ChevronDown className="h-3.5 w-3.5" />
                        )}
                    </button>
                ) : null}
            </div>
            {parsedLaunchInputs.error ? (
                <p
                    data-testid="execution-launch-inputs-schema-error"
                    className="mb-3 rounded-md border border-destructive/30 bg-destructive/10 px-2.5 py-2 text-[11px] text-destructive"
                >
                    {parsedLaunchInputs.error}
                </p>
            ) : null}
            {!launchInputsCollapsed ? (
                <div
                    data-testid="execution-launch-inputs-body"
                    className="max-h-[min(42vh,20rem)] overflow-y-auto overscroll-contain"
                >
                    <div
                        data-testid="execution-launch-inputs-grid"
                        className={`grid gap-x-4 gap-y-3 ${isNarrowViewport ? 'grid-cols-1' : 'grid-cols-1 lg:grid-cols-12'}`}
                    >
                        {parsedLaunchInputs.entries.map((entry, index) => (
                            <div
                                key={`${executionFlowName || 'flow'}-${entry.key}`}
                                data-testid={`execution-launch-input-field-${entry.key}`}
                                className={`space-y-1.5 ${
                                    isNarrowViewport
                                        ? 'col-span-1'
                                        : launchInputDesktopSpanClass(
                                            entry.type,
                                            index,
                                            launchInputCount,
                                            entry.required,
                                        )
                                }`}
                            >
                                <div
                                    className={`border-b border-border/40 pb-1 ${
                                        isNarrowViewport ? 'space-y-1' : 'flex items-start justify-between gap-3'
                                    }`}
                                >
                                    <div className="min-w-0">
                                        <label className="text-xs font-medium text-foreground">
                                            {entry.label}
                                        </label>
                                        {entry.description ? (
                                            <p className="mt-0.5 text-[10px] leading-4 text-muted-foreground">
                                                {entry.description}
                                            </p>
                                        ) : null}
                                    </div>
                                    <p className="shrink-0 text-[10px] leading-4 text-muted-foreground">
                                        {launchInputTypeLabel(entry.type)}
                                        {entry.required ? ' · Required' : ''}
                                    </p>
                                </div>
                                {entry.type === 'string' ? (
                                    <input
                                        data-testid={`execution-launch-input-${entry.key}`}
                                        value={launchInputValues[entry.key] ?? ''}
                                        onChange={(event) => onInputChange(entry, event.target.value)}
                                        className="h-8 w-full rounded-md border border-input bg-background px-2 text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                    />
                                ) : entry.type === 'string[]' ? (
                                    <textarea
                                        data-testid={`execution-launch-input-${entry.key}`}
                                        value={launchInputValues[entry.key] ?? ''}
                                        onChange={(event) => onInputChange(entry, event.target.value)}
                                        rows={2}
                                        className="min-h-16 w-full rounded-md border border-input bg-background px-2 py-1 text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                        placeholder="One item per line"
                                    />
                                ) : entry.type === 'boolean' ? (
                                    <select
                                        data-testid={`execution-launch-input-${entry.key}`}
                                        value={launchInputValues[entry.key] ?? ''}
                                        onChange={(event) => onInputChange(entry, event.target.value)}
                                        className="h-8 w-full rounded-md border border-input bg-background px-2 text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                    >
                                        <option value="">Unset</option>
                                        <option value="true">True</option>
                                        <option value="false">False</option>
                                    </select>
                                ) : entry.type === 'number' ? (
                                    <input
                                        data-testid={`execution-launch-input-${entry.key}`}
                                        type="number"
                                        value={launchInputValues[entry.key] ?? ''}
                                        onChange={(event) => onInputChange(entry, event.target.value)}
                                        className="h-8 w-full rounded-md border border-input bg-background px-2 text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                        placeholder="42"
                                    />
                                ) : (
                                    <textarea
                                        data-testid={`execution-launch-input-${entry.key}`}
                                        value={launchInputValues[entry.key] ?? ''}
                                        onChange={(event) => onInputChange(entry, event.target.value)}
                                        rows={3}
                                        className="min-h-20 w-full rounded-md border border-input bg-background px-2 py-1 text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                        placeholder='{"key":"value"}'
                                    />
                                )}
                            </div>
                        ))}
                    </div>
                </div>
            ) : null}
        </div>
    )
}
