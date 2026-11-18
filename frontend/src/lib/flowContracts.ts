export type LaunchInputType = 'string' | 'string[]' | 'boolean' | 'number' | 'json'

export interface LaunchInputDefinition {
    key: string
    label: string
    type: LaunchInputType
    /** A string written over several lines, such as a bug report: its field keeps Enter for new lines. */
    multiline?: boolean
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
    /** What is wrong with each input, by its key; empty when every input is valid. */
    errors: Record<string, string>
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

// Keys are literal, never trimmed or prefixed: dot-separated identifiers, as the server requires.
const CONTEXT_KEY_PATTERN = /^[A-Za-z0-9_-]+(\.[A-Za-z0-9_-]+)*$/

function keyText(value: unknown): string {
    return typeof value === 'string' ? value : ''
}

function invalidKeyReason(key: string): string | null {
    if (!key) {
        return 'Context keys are required.'
    }
    if (!CONTEXT_KEY_PATTERN.test(key)) {
        return `Context keys must be dot-separated identifiers, with no spaces or empty segments: "${key}"`
    }
    return null
}

function validateContextKey(key: string): string | null {
    const invalid = invalidKeyReason(key)
    if (invalid) {
        return invalid
    }
    if (!key.startsWith('context.')) {
        return `Context keys must use the context.* namespace: ${key}`
    }
    return null
}

// A flow owns its context.* keys; Spark owns the other namespaces the server allows,
// such as parallel.results, tool.output and internal.project_work_dir. The server
// checks which of Spark's keys a step may read.
const CONTRACT_KEY_PREFIXES = ['context.', 'graph.', 'internal.', 'parallel.', 'stack.', 'human.gate.', 'tool.', '_attractor.']

function validateContractKey(key: string): string | null {
    const invalid = invalidKeyReason(key)
    if (invalid) {
        return invalid
    }
    if (!CONTRACT_KEY_PREFIXES.some((prefix) => key.startsWith(prefix))) {
        return `Context keys must use the context.* namespace or one Spark supplies, such as parallel.* or internal.*: ${key}`
    }
    return null
}

export function validateLaunchInputDefinitions(entries: LaunchInputDefinition[]): string | null {
    const seenKeys = new Set<string>()
    for (const entry of entries) {
        const { key } = entry
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
            key: entry.key,
            label: entry.label.trim(),
            type: entry.type,
            ...(entry.type === 'string' && entry.multiline ? { multiline: true } : {}),
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
            const key = keyText(record.key)
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
                ...(record.multiline === true ? { multiline: true } : {}),
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
        const keys = parsed.map(keyText)
        for (const key of keys) {
            const keyError = validateContractKey(key)
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
            keys.filter((key) => key.length > 0),
        ),
    )
    if (normalizedKeys.length === 0) {
        return ''
    }
    return JSON.stringify(normalizedKeys)
}

export function parseContextKeyDraft(value: string): ParsedContextKeyList {
    // A padded line is reported, not trimmed: a key is kept exactly as written.
    const keys = value
        .split(/\r?\n/)
        .filter((entry) => entry.trim().length > 0)
    for (const key of keys) {
        const keyError = validateContractKey(key)
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
    const errors: Record<string, string> = {}

    for (const entry of entries) {
        const rawValue = values[entry.key] ?? ''
        const trimmedValue = rawValue.trim()
        const label = entry.label || entry.key

        if (entry.type === 'string') {
            if (!trimmedValue) {
                if (entry.required) {
                    errors[entry.key] = `${label} is required.`
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
                    errors[entry.key] = `${label} is required.`
                }
                continue
            }
            launchContext[entry.key] = items
            continue
        }

        if (entry.type === 'boolean') {
            if (trimmedValue === '') {
                if (entry.required) {
                    errors[entry.key] = `${label} is required.`
                }
                continue
            }
            if (trimmedValue !== 'true' && trimmedValue !== 'false') {
                errors[entry.key] = `${label} must be true or false.`
                continue
            }
            launchContext[entry.key] = trimmedValue === 'true'
            continue
        }

        if (entry.type === 'number') {
            if (trimmedValue === '') {
                if (entry.required) {
                    errors[entry.key] = `${label} is required.`
                }
                continue
            }
            const numericValue = Number(trimmedValue)
            if (!Number.isFinite(numericValue)) {
                errors[entry.key] = `${label} must be a valid number.`
                continue
            }
            launchContext[entry.key] = numericValue
            continue
        }

        if (entry.type === 'json') {
            if (trimmedValue === '') {
                if (entry.required) {
                    errors[entry.key] = `${label} is required.`
                }
                continue
            }
            try {
                launchContext[entry.key] = JSON.parse(trimmedValue)
            } catch {
                errors[entry.key] = `${label} must be valid JSON.`
            }
        }
    }

    return { launchContext, errors }
}

// Inverse of buildLaunchContextFromValues: map a recorded launch context back
// onto form values so a re-run can prefill the original inputs.
export function launchContextToFormValues(
    entries: LaunchInputDefinition[],
    context: Record<string, unknown>,
): LaunchInputFormValues {
    const values: LaunchInputFormValues = {}
    for (const entry of entries) {
        const raw = context[entry.key]
        if (raw === undefined || raw === null) {
            values[entry.key] = ''
            continue
        }
        if (entry.type === 'string') {
            values[entry.key] = typeof raw === 'string' ? raw : String(raw)
            continue
        }
        if (entry.type === 'string[]') {
            values[entry.key] = Array.isArray(raw)
                ? raw.map((item) => String(item)).join('\n')
                : String(raw)
            continue
        }
        if (entry.type === 'boolean') {
            values[entry.key] = raw === true ? 'true' : raw === false ? 'false' : ''
            continue
        }
        if (entry.type === 'number') {
            values[entry.key] = typeof raw === 'number' && Number.isFinite(raw) ? String(raw) : ''
            continue
        }
        try {
            values[entry.key] = JSON.stringify(raw)
        } catch {
            values[entry.key] = ''
        }
    }
    return values
}

// A `key -> decision` map (switch cases, tool exit codes, subflow results)
// as editable text: one `key = decision` per line. Keys are kept as written,
// so switch case keys keep their JSON scalar type (`0` vs `"0"`).
export function formatDecisionMap(map: unknown): string {
    return Object.entries(asRecord(map) ?? {})
        .map(([key, decision]) => `${key} = ${String(decision)}`)
        .join('\n')
}

// Decisions never contain `=`, so the last `=` separates key from decision.
// A nonblank line without both parts, or a key given twice, is an error: the
// map is never silently reduced.
export function parseDecisionMap(text: string): { map: Record<string, string>; errors: string[] } {
    const map = new Map<string, string>()
    const errors: string[] = []
    text.split('\n').forEach((line, index) => {
        if (!line.trim()) return
        const separator = line.lastIndexOf('=')
        const key = separator < 0 ? '' : line.slice(0, separator).trim()
        const decision = separator < 0 ? '' : line.slice(separator + 1).trim()
        if (!key || !decision) {
            errors.push(`Line ${index + 1} needs \`key = decision\`.`)
        } else if (map.has(key)) {
            errors.push(`Line ${index + 1} repeats key ${key}.`)
        } else {
            map.set(key, decision)
        }
    })
    return { map: Object.fromEntries(map), errors }
}
