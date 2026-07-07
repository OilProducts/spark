export type LaunchInputType = 'string' | 'string[]' | 'boolean' | 'number' | 'json'

export interface LaunchInputDefinition {
    key: string
    label: string
    type: LaunchInputType
    description: string
    required: boolean
}

export interface ParsedLaunchInputDefinitions {
    entries: LaunchInputDefinition[]
    error: string | null
}

export interface ParsedContextKeyList {
    keys: string[]
    error: string | null
}

export interface LaunchContextBuildResult {
    launchContext: Record<string, unknown>
    errors: string[]
}

export type LaunchInputFormValues = Record<string, string>

const LAUNCH_INPUT_TYPES: LaunchInputType[] = ['string', 'string[]', 'boolean', 'number', 'json']

function asRecord(value: unknown): Record<string, unknown> | null {
    if (!value || typeof value !== 'object' || Array.isArray(value)) {
        return null
    }
    return value as Record<string, unknown>
}

function isLaunchInputType(value: unknown): value is LaunchInputType {
    return typeof value === 'string' && LAUNCH_INPUT_TYPES.includes(value as LaunchInputType)
}

function normalizeContextKey(value: unknown): string {
    return typeof value === 'string' ? value.trim() : ''
}

function validateContextKey(key: string): string | null {
    if (!key) {
        return 'Context keys are required.'
    }
    if (!key.startsWith('context.')) {
        return `Context keys must use the context.* namespace: ${key}`
    }
    return null
}

export function validateLaunchInputDefinitions(entries: LaunchInputDefinition[]): string | null {
    const seenKeys = new Set<string>()
    for (const entry of entries) {
        const key = normalizeContextKey(entry.key)
        const keyError = validateContextKey(key)
        if (keyError) {
            return keyError
        }
        if (seenKeys.has(key)) {
            return `Launch input keys must be unique: ${key}`
        }
        seenKeys.add(key)
        if (!isLaunchInputType(entry.type)) {
            return `Unsupported launch input type: ${String(entry.type)}`
        }
        if (!entry.label.trim()) {
            return `Launch input labels are required for ${key}`
        }
    }
    return null
}

export function serializeLaunchInputDefinitions(entries: LaunchInputDefinition[]): string {
    const normalizedEntries = entries
        .map((entry) => ({
            key: normalizeContextKey(entry.key),
            label: entry.label.trim(),
            type: entry.type,
            description: entry.description.trim(),
            required: Boolean(entry.required),
        }))
        .filter((entry) => entry.key.length > 0)
    if (normalizedEntries.length === 0) {
        return ''
    }
    return JSON.stringify(normalizedEntries)
}

export function parseLaunchInputDefinitions(raw: unknown): ParsedLaunchInputDefinitions {
    const parsedRaw = Array.isArray(raw) ? raw : null
    const rawText = typeof raw === 'string' ? raw.trim() : ''
    if (!parsedRaw && !rawText) {
        return { entries: [], error: null }
    }
    try {
        const parsed = parsedRaw ?? JSON.parse(rawText)
        if (!Array.isArray(parsed)) {
            return { entries: [], error: 'Launch inputs must be a JSON array.' }
        }
        const entries: LaunchInputDefinition[] = []
        for (const item of parsed) {
            const record = asRecord(item)
            if (!record) {
                return { entries: [], error: 'Each launch input must be an object.' }
            }
            const key = normalizeContextKey(record.key)
            const label = typeof record.label === 'string' ? record.label.trim() : ''
            const type = record.type
            const description = typeof record.description === 'string' ? record.description.trim() : ''
            const required = record.required === true
            if (!isLaunchInputType(type)) {
                return { entries: [], error: `Unsupported launch input type: ${String(type)}` }
            }
            entries.push({
                key,
                label,
                type,
                description,
                required,
            })
        }
        const validationError = validateLaunchInputDefinitions(entries)
        return validationError
            ? { entries: [], error: validationError }
            : { entries, error: null }
    } catch {
        return { entries: [], error: 'Launch inputs must be valid JSON.' }
    }
}

export function parseContextKeyList(raw: unknown): ParsedContextKeyList {
    const rawText = typeof raw === 'string' ? raw.trim() : ''
    if (!rawText) {
        return { keys: [], error: null }
    }
    try {
        const parsed = JSON.parse(rawText)
        if (!Array.isArray(parsed)) {
            return { keys: [], error: 'Context key declarations must be a JSON array.' }
        }
        const keys = parsed.map((value) => normalizeContextKey(value)).filter((value) => value.length > 0)
        for (const key of keys) {
            const keyError = validateContextKey(key)
            if (keyError) {
                return { keys: [], error: keyError }
            }
        }
        return { keys: Array.from(new Set(keys)), error: null }
    } catch {
        return { keys: [], error: 'Context key declarations must be valid JSON.' }
    }
}

export function serializeContextKeyList(keys: string[]): string {
    const normalizedKeys = Array.from(
        new Set(
            keys
                .map((key) => normalizeContextKey(key))
                .filter((key) => key.length > 0),
        ),
    )
    if (normalizedKeys.length === 0) {
        return ''
    }
    return JSON.stringify(normalizedKeys)
}

export function parseContextKeyDraft(value: string): ParsedContextKeyList {
    const keys = value
        .split(/\r?\n/)
        .map((entry) => entry.trim())
        .filter((entry) => entry.length > 0)
    for (const key of keys) {
        const keyError = validateContextKey(key)
        if (keyError) {
            return { keys, error: keyError }
        }
    }
    return { keys: Array.from(new Set(keys)), error: null }
}

export function initializeLaunchInputFormValues(
    entries: LaunchInputDefinition[],
    previousValues?: LaunchInputFormValues,
): LaunchInputFormValues {
    const nextValues: LaunchInputFormValues = {}
    for (const entry of entries) {
        nextValues[entry.key] = previousValues?.[entry.key] ?? ''
    }
    return nextValues
}

export function buildLaunchContextFromValues(
    entries: LaunchInputDefinition[],
    values: LaunchInputFormValues,
): LaunchContextBuildResult {
    const launchContext: Record<string, unknown> = {}
    const errors: string[] = []

    for (const entry of entries) {
        const rawValue = values[entry.key] ?? ''
        const trimmedValue = rawValue.trim()
        const label = entry.label || entry.key

        if (entry.type === 'string') {
            if (!trimmedValue) {
                if (entry.required) {
                    errors.push(`${label} is required.`)
                }
                continue
            }
            launchContext[entry.key] = trimmedValue
            continue
        }

        if (entry.type === 'string[]') {
            const items = rawValue
                .split(/\r?\n/)
                .map((item) => item.trim())
                .filter((item) => item.length > 0)
            if (items.length === 0) {
                if (entry.required) {
                    errors.push(`${label} is required.`)
                }
                continue
            }
            launchContext[entry.key] = items
            continue
        }

        if (entry.type === 'boolean') {
            if (trimmedValue === '') {
                if (entry.required) {
                    errors.push(`${label} is required.`)
                }
                continue
            }
            if (trimmedValue !== 'true' && trimmedValue !== 'false') {
                errors.push(`${label} must be true or false.`)
                continue
            }
            launchContext[entry.key] = trimmedValue === 'true'
            continue
        }

        if (entry.type === 'number') {
            if (trimmedValue === '') {
                if (entry.required) {
                    errors.push(`${label} is required.`)
                }
                continue
            }
            const numericValue = Number(trimmedValue)
            if (!Number.isFinite(numericValue)) {
                errors.push(`${label} must be a valid number.`)
                continue
            }
            launchContext[entry.key] = numericValue
            continue
        }

        if (entry.type === 'json') {
            if (trimmedValue === '') {
                if (entry.required) {
                    errors.push(`${label} is required.`)
                }
                continue
            }
            try {
                launchContext[entry.key] = JSON.parse(trimmedValue)
            } catch {
                errors.push(`${label} must be valid JSON.`)
            }
        }
    }

    return { launchContext, errors }
}
