export type LlmProviderKey = 'openai' | 'anthropic' | 'gemini' | 'mistral'

export const LLM_PROVIDER_OPTIONS: LlmProviderKey[] = ['openai', 'anthropic', 'gemini', 'mistral']

export const LLM_MODELS_BY_PROVIDER: Record<LlmProviderKey, string[]> = {
    openai: [
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
    mistral: ['mistral-large-2512'],
}

export function getModelSuggestions(provider?: string): string[] {
    const normalized = (provider || '').trim().toLowerCase() as LlmProviderKey
    if (normalized && LLM_MODELS_BY_PROVIDER[normalized]) {
        return LLM_MODELS_BY_PROVIDER[normalized]
    }
    return Array.from(new Set(Object.values(LLM_MODELS_BY_PROVIDER).flat()))
}
