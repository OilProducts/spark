import type { GraphAttrs } from '../store'

const GRAPH_ATTR_STRING_KEYS: (keyof GraphAttrs)[] = [
    'goal',
    'label',
    'retry_target',
    'fallback_retry_target',
    'default_fidelity',
    'stack.child_dotfile',
    'stack.child_workdir',
    'tool_hooks.pre',
    'tool_hooks.post',
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
    if (key === 'default_max_retry') {
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
    if (key === 'default_max_retry') {
        if (!value) return null
        if (!/^\d+$/.test(value)) {
            return 'Default max retry must be a non-negative integer.'
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
