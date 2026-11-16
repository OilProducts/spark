import type { ProjectChatModelProviderStatusResponse } from '@/lib/api/projectsApi'

export type LlmProviderKey = 'codex' | 'claude-code' | 'openai' | 'anthropic' | 'gemini' | 'openrouter' | 'litellm' | 'openai_compatible'

export const LLM_PROVIDER_OPTIONS: LlmProviderKey[] = ['codex', 'claude-code', 'openai', 'anthropic', 'gemini', 'openrouter', 'litellm', 'openai_compatible']

export interface LlmProfileMetadata {
    id: string
    label?: string | null
    provider: string
    models: string[]
    reasoning_efforts?: string[]
    default_model?: string | null
    configured: boolean
}

export const LLM_MODELS_BY_PROVIDER: Record<LlmProviderKey, string[]> = {
    codex: [],
    // Populated live from the CLI's model catalog via the chat-models
    // endpoint; no static fallback because a missing CLI cannot run turns.
    'claude-code': [],
    openai: [
        'gpt-5.5',
        'gpt-5.4',
        'gpt-5.2',
        'gpt-5.2-pro',
        'gpt-5.2-chat-latest',
        'gpt-5',
        'gpt-5-mini',
        'gpt-5-nano',
        'gpt-4.1',
        'gpt-oss-120b',
        'gpt-oss-20b',
    ],
    anthropic: [
        'claude-opus-4-6',
        'claude-sonnet-4-6',
        'claude-sonnet-4-20250514',
    ],
    gemini: [
        'gemini-2.5-flash',
        'gemini-2.5-flash-preview-09-2025',
        'gemini-flash-latest',
    ],
    openrouter: [
        'openai/gpt-5.4',
        'anthropic/claude-sonnet-4.5',
        'google/gemini-2.5-flash',
    ],
    litellm: [],
    openai_compatible: [],
}

export function getModelSuggestions(provider?: string, profiles: LlmProfileMetadata[] = []): string[] {
    const profile = profiles.find((entry) => entry.id === provider)
    if (profile) {
        return [...profile.models]
    }
    const normalized = (provider || '').trim().toLowerCase() as LlmProviderKey
    if (!normalized) {
        return Array.from(new Set(Object.values(LLM_MODELS_BY_PROVIDER).flat()))
    }
    // A discovered provider, such as an ACP harness, lists only what it reports.
    return LLM_MODELS_BY_PROVIDER[normalized] ?? []
}

/** The built-in providers, then discovered ones such as ACP harnesses, then profiles. */
export function getLlmSelectionOptions(profiles: LlmProfileMetadata[] = [], discovered: string[] = []): string[] {
    return [...new Set([...LLM_PROVIDER_OPTIONS, ...discovered]), ...profiles.map((profile) => profile.id)]
}

/** A provider's name as discovery reports it, else its selector. */
export function providerLabel(provider: string, statuses?: Record<string, ProjectChatModelProviderStatusResponse>): string {
    return statuses?.[provider]?.label || provider
}

export interface SessionControlValue {
    value: string
    name: string
    group?: string
}

export interface SessionControl {
    id: string
    name: string
    description?: string
    values: SessionControlValue[]
}

type RawOption = Record<string, unknown>
const record = (value: unknown): RawOption | null => value && typeof value === 'object' && !Array.isArray(value) ? value as RawOption : null
const text = (value: unknown) => typeof value === 'string' ? value : undefined
const selectOf = (options: RawOption[], category: string) => options.find((option) => option.category === category && option.type === 'select')

/** A select option's values in order, with grouped values flattened under their group's name. */
function selectValues(option: RawOption): SessionControlValue[] {
    return (Array.isArray(option.options) ? option.options : []).map(record).flatMap((entry): SessionControlValue[] => {
        if (!entry) return []
        if (Array.isArray(entry.options)) {
            return entry.options.map(record).flatMap((value) => value && text(value.value) ? [{ value: text(value.value)!, name: text(value.name) || text(value.value)!, group: text(entry.name) || text(entry.group) }] : [])
        }
        return text(entry.value) ? [{ value: text(entry.value)!, name: text(entry.name) || text(entry.value)! }] : []
    })
}

/**
 * A harness's editable session controls, by exact config ID: its select options
 * other than the model and thought-level selectors, which map to Spark's model
 * and effort. Boolean and unknown option types keep the harness default.
 */
export function sessionControls(status?: ProjectChatModelProviderStatusResponse): SessionControl[] {
    const options = (status?.config_options ?? []).map(record).filter((option): option is RawOption => !!option)
    const mapped = [selectOf(options, 'model'), selectOf(options, 'thought_level')]
    return options.flatMap((option) => option.type !== 'select' || mapped.includes(option) || !text(option.id) ? [] : [{
        id: text(option.id)!,
        name: text(option.name) || text(option.id)!,
        description: text(option.description),
        values: selectValues(option),
    }])
}

/** The harness's own names for its thought-level value IDs. */
export function effortNames(status?: ProjectChatModelProviderStatusResponse): Record<string, string> {
    const options = (status?.config_options ?? []).map(record).filter((option): option is RawOption => !!option)
    const option = selectOf(options, 'thought_level')
    return Object.fromEntries((option ? selectValues(option) : []).map((value) => [value.value, value.name]))
}

export function splitLlmSelection(value: string, profiles: LlmProfileMetadata[]): { llm_provider: string; llm_profile: string } {
    const selection = value.trim()
    if (profiles.some((profile) => profile.id === selection)) {
        return {
            llm_provider: '',
            llm_profile: selection,
        }
    }
    return {
        llm_provider: selection,
        llm_profile: '',
    }
}

export function isModelSelectionValid(provider: string, model: string | null, profiles: LlmProfileMetadata[]): boolean {
    const profile = profiles.find((entry) => entry.id === provider)
    return profile ? (model ? profile.models.includes(model) : !!profile.default_model)
        : !['openrouter', 'litellm', 'openai_compatible'].includes(provider) || !!model
}
