export type LlmProviderKey = 'codex' | 'openai' | 'anthropic' | 'gemini' | 'openrouter' | 'litellm' | 'openai_compatible'

export const LLM_PROVIDER_OPTIONS: LlmProviderKey[] = ['codex', 'openai', 'anthropic', 'gemini', 'openrouter', 'litellm', 'openai_compatible']

export interface LlmProfileMetadata {
    id: string
    label?: string | null
    provider: string
    models: string[]
    default_model?: string | null
    configured: boolean
}

export const LLM_MODELS_BY_PROVIDER: Record<LlmProviderKey, string[]> = {
    codex: [
        'gpt-5.4',
        'gpt-5.4-mini',
        'gpt-5.2',
    ],
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
    if (normalized && LLM_MODELS_BY_PROVIDER[normalized]) {
        return LLM_MODELS_BY_PROVIDER[normalized]
    }
    return Array.from(new Set(Object.values(LLM_MODELS_BY_PROVIDER).flat()))
}

export function getLlmSelectionOptions(profiles: LlmProfileMetadata[] = []): string[] {
    return [...LLM_PROVIDER_OPTIONS, ...profiles.map((profile) => profile.id)]
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
