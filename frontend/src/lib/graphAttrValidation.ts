import type { GraphAttrs } from '../store'

const GRAPH_ATTR_STRING_KEYS: (keyof GraphAttrs)[] = [
    'spark.title',
    'spark.description',
    'goal',
    'label',
    'retry_target',
    'fallback_retry_target',
    'default_fidelity',
    'stack.child_dotfile',
    'stack.child_workdir',
    'tool.hooks.pre',
    'tool.hooks.post',
    'ui_default_llm_model',
    'ui_default_llm_provider',
    'ui_default_reasoning_effort',
]

export const GRAPH_FIDELITY_OPTIONS = [
    'full',
    'truncate',
    'compact',
    'summary:low',
    'summary:medium',
    'summary:high',
] as const

const GRAPH_FIDELITY_OPTION_SET = new Set<string>(GRAPH_FIDELITY_OPTIONS)

export const normalizeGraphAttrValue = (key: keyof GraphAttrs, value: string): string => {
    if (key === 'model_stylesheet') {
        return value
    }
    if (key === 'default_max_retries') {
        const trimmed = value.trim()
        if (!trimmed) return ''
        if (!/^\d+$/.test(trimmed)) return trimmed
        return `${Math.max(0, parseInt(trimmed, 10))}`
    }
    if (key === 'default_fidelity') {
        return value.trim().toLowerCase()
    }
    if (GRAPH_ATTR_STRING_KEYS.includes(key)) {
        return value.trim()
    }
    return value
}

export const validateGraphAttrValue = (key: keyof GraphAttrs, value: string): string | null => {
    if (key === 'default_max_retries') {
        if (!value) return null
        if (!/^\d+$/.test(value)) {
            return 'Default max retries must be a non-negative integer.'
        }
        return null
    }
    if (key === 'default_fidelity') {
        if (!value) return null
        if (!GRAPH_FIDELITY_OPTION_SET.has(value)) {
            return 'Default fidelity must be one of: full, truncate, compact, summary:low, summary:medium, summary:high.'
        }
        return null
    }
    return null
}

const detectUnmatchedQuotes = (value: string): { single: boolean; double: boolean } => {
    let inSingle = false
    let inDouble = false
    let escaped = false
    for (const char of value) {
        if (escaped) {
            escaped = false
            continue
        }
        if (char === '\\' && inDouble) {
            escaped = true
            continue
        }
        if (char === "'" && !inDouble) {
            inSingle = !inSingle
            continue
        }
        if (char === '"' && !inSingle) {
            inDouble = !inDouble
            continue
        }
    }
    return { single: inSingle, double: inDouble }
}

export const getToolHookCommandWarning = (value: string): string | null => {
    const trimmed = value.trim()
    if (!trimmed) {
        return null
    }
    if (/[\r\n]/.test(trimmed)) {
        return 'Tool hook command should be a single line shell command.'
    }
    const unmatchedQuotes = detectUnmatchedQuotes(trimmed)
    if (unmatchedQuotes.single) {
        return 'Tool hook command appears malformed: unmatched single quote.'
    }
    if (unmatchedQuotes.double) {
        return 'Tool hook command appears malformed: unmatched double quote.'
    }
    return null
}
