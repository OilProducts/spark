import type { LaunchInputDefinition, LaunchInputType } from '@/lib/flowContracts'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import { Field, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { NativeSelect } from '@/components/ui/native-select'
import { Textarea } from '@/components/ui/textarea'

const LAUNCH_INPUT_TYPE_OPTIONS: Array<{ value: LaunchInputType; label: string }> = [
    { value: 'string', label: 'String' },
    { value: 'string[]', label: 'String List' },
    { value: 'boolean', label: 'Boolean' },
    { value: 'number', label: 'Number' },
    { value: 'json', label: 'JSON' },
]

interface LaunchInputsEditorProps {
    entries: LaunchInputDefinition[]
    error: string | null
    onChange: (entries: LaunchInputDefinition[]) => void
}

export function LaunchInputsEditor({ entries, error, onChange }: LaunchInputsEditorProps) {
    const updateEntry = (index: number, patch: Partial<LaunchInputDefinition>) => {
        onChange(entries.map((entry, entryIndex) => (
            entryIndex === index ? { ...entry, ...patch } : entry
        )))
    }

    const removeEntry = (index: number) => {
        onChange(entries.filter((_, entryIndex) => entryIndex !== index))
    }

    const addEntry = () => {
        onChange([
            ...entries,
            {
                key: 'context.request.',
                label: '',
                type: 'string',
                description: '',
                required: false,
            },
        ])
    }

    return (
        <div
            data-testid="graph-launch-inputs-editor"
            className="space-y-3 rounded-md border border-border/80 bg-background/40 p-3"
        >
            <div className="space-y-1">
                <p className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
                    Launch Inputs
                </p>
                <p className="text-[11px] text-muted-foreground">
                    Define the `context.*` values Spark should collect before launching this flow.
                </p>
            </div>
            {entries.length === 0 ? (
                <div className="rounded-md border border-dashed border-border/80 bg-muted/10 px-3 py-2 text-[11px] text-muted-foreground">
                    No launch inputs declared yet.
                </div>
            ) : null}
            {entries.map((entry, index) => (
                <div
                    key={`launch-input-${index}`}
                    data-testid={`graph-launch-input-row-${index}`}
                    className="space-y-3 rounded-md border border-border/80 bg-background px-3 py-3"
                >
                    <div className="grid grid-cols-2 gap-3">
                        <Field>
                            <FieldLabel>Label</FieldLabel>
                            <Input
                                data-testid={`graph-launch-input-label-${index}`}
                                value={entry.label}
                                onChange={(event) => updateEntry(index, { label: event.target.value })}
                                className="h-8 px-2 text-xs"
                                placeholder="Request Summary"
                            />
                        </Field>
                        <Field>
                            <FieldLabel>Type</FieldLabel>
                            <NativeSelect
                                data-testid={`graph-launch-input-type-${index}`}
                                value={entry.type}
                                onChange={(event) => updateEntry(index, { type: event.target.value as LaunchInputType })}
                                className="h-8 px-2 text-xs"
                            >
                                {LAUNCH_INPUT_TYPE_OPTIONS.map((option) => (
                                    <option key={option.value} value={option.value}>
                                        {option.label}
                                    </option>
                                ))}
                            </NativeSelect>
                        </Field>
                    </div>
                    <Field>
                        <FieldLabel>Context Key</FieldLabel>
                        <Input
                            data-testid={`graph-launch-input-key-${index}`}
                            value={entry.key}
                            onChange={(event) => updateEntry(index, { key: event.target.value })}
                            className="h-8 px-2 font-mono text-xs"
                            placeholder="context.request.summary"
                        />
                    </Field>
                    <Field>
                        <FieldLabel>Description</FieldLabel>
                        <Textarea
                            data-testid={`graph-launch-input-description-${index}`}
                            value={entry.description}
                            onChange={(event) => updateEntry(index, { description: event.target.value })}
                            rows={2}
                            className="min-h-16 px-2 py-1 text-xs"
                            placeholder="Short explanation shown in the launch form."
                        />
                    </Field>
                    <div className="flex items-center justify-between gap-3">
                        <Label className="inline-flex items-center gap-2 text-xs font-medium text-foreground">
                            <Checkbox
                                data-testid={`graph-launch-input-required-${index}`}
                                checked={entry.required}
                                onCheckedChange={(checked) => updateEntry(index, { required: checked === true })}
                            />
                            Required at launch
                        </Label>
                        <Button
                            type="button"
                            data-testid={`graph-launch-input-remove-${index}`}
                            onClick={() => removeEntry(index)}
                            variant="outline"
                            size="xs"
                            className="text-[11px] text-muted-foreground hover:text-foreground"
                        >
                            Remove
                        </Button>
                    </div>
                </div>
            ))}
            <div className="flex items-center justify-between gap-3">
                <Button
                    type="button"
                    data-testid="graph-launch-input-add"
                    onClick={addEntry}
                    variant="outline"
                    size="xs"
                    className="h-8 text-[11px] uppercase tracking-wide text-muted-foreground hover:text-foreground"
                >
                    Add Launch Input
                </Button>
                {error ? (
                    <p data-testid="graph-launch-inputs-error" className="text-[11px] text-destructive">
                        {error}
                    </p>
                ) : (
                    <p className="text-[11px] text-muted-foreground">
                        Keys must use the `context.*` namespace.
                    </p>
                )}
            </div>
        </div>
    )
}
