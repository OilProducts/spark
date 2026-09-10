import { useId } from 'react'
import { Field, FieldLabel } from '@/components/ui/field'
import { Textarea } from '@/components/ui/textarea'
interface ContextKeyListEditorProps {
    title: string
    description: string
    value: string
    error: string | null
    testId: string
    onChange: (value: string) => void
}

export function ContextKeyListEditor({
    title,
    description,
    value,
    error,
    testId,
    onChange,
}: ContextKeyListEditorProps) {
    const id = useId()
    return (
        <div data-testid={testId} className="space-y-1.5 rounded-md border border-border/80 bg-muted/10 px-3 py-3">
            <div>
                <FieldLabel htmlFor={`${id}-keys`}>{title}</FieldLabel>
                <p id={`${id}-description`} className="mt-1 text-[11px] text-muted-foreground">{description}</p>
            </div>
            <Field className="gap-1">
                <Textarea
                    id={`${id}-keys`}
                    aria-describedby={`${id}-description ${id}-${error ? 'error' : 'help'}`}
                    aria-invalid={Boolean(error) || undefined}
                    data-testid={`${testId}-textarea`}
                    value={value}
                    onChange={(event) => onChange(event.target.value)}
                    rows={4}
                    className="min-h-24 px-2 py-2 font-mono text-xs"
                    placeholder="One context.* key per line"
                />
            </Field>
            {error ? (
                <p id={`${id}-error`} data-testid={`${testId}-error`} className="text-[11px] text-destructive">
                    {error}
                </p>
            ) : (
                <p id={`${id}-help`} className="text-[11px] text-muted-foreground">One `context.*` key per line.</p>
            )}
        </div>
    )
}
