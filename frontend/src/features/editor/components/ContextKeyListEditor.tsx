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
    return (
        <div data-testid={testId} className="space-y-1.5 rounded-md border border-border/80 bg-muted/10 px-3 py-3">
            <div>
                <p className="text-sm font-medium text-foreground">{title}</p>
                <p className="mt-1 text-[11px] text-muted-foreground">{description}</p>
            </div>
            <Field className="gap-1">
                <FieldLabel>Context Keys</FieldLabel>
                <Textarea
                    data-testid={`${testId}-textarea`}
                    value={value}
                    onChange={(event) => onChange(event.target.value)}
                    rows={4}
                    className="min-h-24 px-2 py-2 font-mono text-xs"
                    placeholder="One context.* key per line"
                />
            </Field>
            {error ? (
                <p data-testid={`${testId}-error`} className="text-[11px] text-destructive">
                    {error}
                </p>
            ) : (
                <p className="text-[11px] text-muted-foreground">One `context.*` key per line.</p>
            )}
        </div>
    )
}
