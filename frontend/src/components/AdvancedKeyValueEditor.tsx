import { useMemo, useState } from 'react'

import type { ExtensionAttrEntry } from '@/lib/extensionAttrs'

interface AdvancedKeyValueEditorProps {
    testIdPrefix: string
    entries: ExtensionAttrEntry[]
    onValueChange: (key: string, value: string) => void
    onRemove: (key: string) => void
    onAdd: (key: string, value: string) => void
    reservedKeys?: Set<string>
    title?: string
}

export function AdvancedKeyValueEditor({
    testIdPrefix,
    entries,
    onValueChange,
    onRemove,
    onAdd,
    reservedKeys,
    title = 'Extension Attributes',
}: AdvancedKeyValueEditorProps) {
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
            <p className="text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">
                {title}
            </p>
            <p className="text-[11px] text-muted-foreground">
                Edit non-core attributes as generic key/value pairs.
            </p>

            {entries.length === 0 ? (
                <p
                    data-testid={`${testIdPrefix}-extension-attrs-empty`}
                    className="text-[11px] text-muted-foreground"
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
                            <label className="space-y-1">
                                <span className="text-[11px] font-medium text-muted-foreground">Key</span>
                                <input
                                    data-testid={`${testIdPrefix}-extension-attr-key-${index}`}
                                    value={entry.key}
                                    readOnly
                                    className="h-8 w-full rounded-md border border-input bg-muted/30 px-2 font-mono text-[11px] text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                />
                            </label>
                            <label className="space-y-1">
                                <span className="text-[11px] font-medium text-muted-foreground">Value</span>
                                <input
                                    data-testid={`${testIdPrefix}-extension-attr-value-${index}`}
                                    value={entry.value}
                                    onChange={(event) => onValueChange(entry.key, event.target.value)}
                                    className="h-8 w-full rounded-md border border-input bg-background px-2 font-mono text-[11px] text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                />
                            </label>
                            <button
                                type="button"
                                data-testid={`${testIdPrefix}-extension-attr-remove-${index}`}
                                onClick={() => onRemove(entry.key)}
                                className="h-8 rounded-md border border-border px-2 text-[11px] font-semibold uppercase tracking-wide text-muted-foreground hover:text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                            >
                                Remove
                            </button>
                        </div>
                    ))}
                </div>
            )}

            <div className="grid grid-cols-[1fr_1fr_auto] items-end gap-2">
                <label className="space-y-1">
                    <span className="text-[11px] font-medium text-muted-foreground">New Key</span>
                    <input
                        data-testid={`${testIdPrefix}-extension-attr-new-key`}
                        value={newKey}
                        onChange={(event) => setNewKey(event.target.value)}
                        className="h-8 w-full rounded-md border border-input bg-background px-2 font-mono text-[11px] text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                        placeholder="x_custom_attr"
                    />
                </label>
                <label className="space-y-1">
                    <span className="text-[11px] font-medium text-muted-foreground">New Value</span>
                    <input
                        data-testid={`${testIdPrefix}-extension-attr-new-value`}
                        value={newValue}
                        onChange={(event) => setNewValue(event.target.value)}
                        className="h-8 w-full rounded-md border border-input bg-background px-2 font-mono text-[11px] text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                        placeholder="value"
                    />
                </label>
                <button
                    type="button"
                    onClick={handleAdd}
                    disabled={!canAdd}
                    className="h-8 rounded-md border border-border px-2 text-[11px] font-semibold uppercase tracking-wide text-muted-foreground hover:text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50"
                >
                    Add Attribute
                </button>
            </div>
            {hasDuplicateKey ? (
                <p className="text-[11px] text-amber-700">
                    Key already exists.
                </p>
            ) : null}
            {hasReservedKey ? (
                <p className="text-[11px] text-amber-700">
                    Core attributes belong in dedicated controls.
                </p>
            ) : null}
        </section>
    )
}
