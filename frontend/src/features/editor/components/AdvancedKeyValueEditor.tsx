import { useId, useMemo, useState } from 'react'

import type { ExtensionAttrEntry } from '@/lib/extensionAttrs'
import { Button } from '@/components/ui/button'
import { Field, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'

interface AdvancedKeyValueEditorProps {
    testIdPrefix: string
    entries: ExtensionAttrEntry[]
    onValueChange: (key: string, value: string) => void
    onRemove: (key: string) => void
    onAdd: (key: string, value: string) => void
    reservedKeys?: Set<string>
    title?: string
    description?: string
}

export function AdvancedKeyValueEditor({
    testIdPrefix,
    entries,
    onValueChange,
    onRemove,
    onAdd,
    reservedKeys,
    title = 'Extension Attributes',
    description = 'Edit non-core attributes as generic key/value pairs.',
}: AdvancedKeyValueEditorProps) {
    const id = useId()
    const [newKey, setNewKey] = useState('')
    const [newValue, setNewValue] = useState('')
    const normalizedNewKey = newKey.trim()
    const hasDuplicateKey = useMemo(
        () => entries.some((entry) => entry.key === normalizedNewKey),
        [entries, normalizedNewKey],
    )
    const hasReservedKey = Boolean(normalizedNewKey && reservedKeys?.has(normalizedNewKey))
    const canAdd = normalizedNewKey.length > 0 && !hasDuplicateKey && !hasReservedKey

    const handleAdd = () => {
        if (!canAdd) {
            return
        }
        onAdd(normalizedNewKey, newValue)
        setNewKey('')
        setNewValue('')
    }

    return (
        <section
            data-testid={`${testIdPrefix}-extension-attrs-editor`}
            className="space-y-2 rounded-md border border-border/80 bg-muted/10 p-3"
        >
            <p className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
                {title}
            </p>
            <p className="text-xs text-muted-foreground">
                {description}
            </p>

            {entries.length === 0 ? (
                <p
                    data-testid={`${testIdPrefix}-extension-attrs-empty`}
                    className="text-xs text-muted-foreground"
                >
                    No extension attributes set.
                </p>
            ) : (
                <div
                    data-testid={`${testIdPrefix}-extension-attrs-list`}
                    className="space-y-2"
                >
                    {entries.map((entry, index) => (
                        <div key={entry.key} className="grid grid-cols-[1fr_1fr_auto] items-end gap-2">
                            <Field className="gap-1">
                                <FieldLabel htmlFor={`${id}-extension-attr-key-input-${index}`}>
                                    Key
                                </FieldLabel>
                                <Input
                                    id={`${id}-extension-attr-key-input-${index}`}
                                    data-testid={`${testIdPrefix}-extension-attr-key-${index}`}
                                    value={entry.key}
                                    readOnly
                                    className="h-8 bg-muted/30 px-2 font-mono text-sm"
                                />
                            </Field>
                            <Field className="gap-1">
                                <FieldLabel htmlFor={`${id}-extension-attr-value-input-${index}`}>
                                    Value
                                </FieldLabel>
                                <Input
                                    id={`${id}-extension-attr-value-input-${index}`}
                                    data-testid={`${testIdPrefix}-extension-attr-value-${index}`}
                                    value={entry.value}
                                    onChange={(event) => onValueChange(entry.key, event.target.value)}
                                    className="h-8 px-2 font-mono text-sm"
                                />
                            </Field>
                            <Button
                                type="button"
                                data-testid={`${testIdPrefix}-extension-attr-remove-${index}`}
                                onClick={() => onRemove(entry.key)}
                                variant="outline"
                                size="xs"
                                className="h-8 text-xs uppercase tracking-wide text-muted-foreground hover:text-foreground"
                            >
                                Remove
                            </Button>
                        </div>
                    ))}
                </div>
            )}

            <div className="grid grid-cols-[1fr_1fr_auto] items-end gap-2">
                <Field className="gap-1">
                    <FieldLabel htmlFor={`${id}-extension-attr-new-key-input`}>
                        New Key
                    </FieldLabel>
                    <Input
                        id={`${id}-extension-attr-new-key-input`}
                        aria-describedby={[hasDuplicateKey && `${id}-duplicate-warning`, hasReservedKey && `${id}-reserved-warning`].filter(Boolean).join(' ') || undefined}
                        data-testid={`${testIdPrefix}-extension-attr-new-key`}
                        value={newKey}
                        onChange={(event) => setNewKey(event.target.value)}
                        className="h-8 px-2 font-mono text-sm"
                        placeholder="x_custom_attr"
                    />
                </Field>
                <Field className="gap-1">
                    <FieldLabel htmlFor={`${id}-extension-attr-new-value-input`}>
                        New Value
                    </FieldLabel>
                    <Input
                        id={`${id}-extension-attr-new-value-input`}
                        data-testid={`${testIdPrefix}-extension-attr-new-value`}
                        value={newValue}
                        onChange={(event) => setNewValue(event.target.value)}
                        className="h-8 px-2 font-mono text-sm"
                        placeholder="value"
                    />
                </Field>
                <Button
                    type="button"
                    onClick={handleAdd}
                    disabled={!canAdd}
                    variant="outline"
                    size="xs"
                    className="h-8 text-xs uppercase tracking-wide text-muted-foreground hover:text-foreground"
                >
                    Add Attribute
                </Button>
            </div>
            {hasDuplicateKey ? (
                <p id={`${id}-duplicate-warning`} className="text-xs text-warning">
                    Key already exists.
                </p>
            ) : null}
            {hasReservedKey ? (
                <p id={`${id}-reserved-warning`} className="text-xs text-warning">
                    Core attributes belong in dedicated controls.
                </p>
            ) : null}
        </section>
    )
}
