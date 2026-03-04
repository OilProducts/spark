export interface ExtensionAttrEntry {
    key: string
    value: string
}

const stringifyExtensionAttrValue = (value: unknown): string => {
    if (value === null || value === undefined) {
        return ''
    }
    if (typeof value === 'string' || typeof value === 'number' || typeof value === 'boolean') {
        return String(value)
    }
    return ''
}

export function toExtensionAttrEntries(
    attrs: Record<string, unknown> | null | undefined,
    coreKeys: Set<string>,
    excludedKeys?: Set<string>,
): ExtensionAttrEntry[] {
    if (!attrs) {
        return []
    }
    return Object.entries(attrs)
        .filter(([key]) => !coreKeys.has(key) && !excludedKeys?.has(key))
        .map(([key, value]) => ({
            key,
            value: stringifyExtensionAttrValue(value),
        }))
        .sort((left, right) => left.key.localeCompare(right.key))
}
