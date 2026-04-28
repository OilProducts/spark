import type { LlmProfileMetadata } from '@/lib/llmSuggestions'
import { ATTRACTOR_API_BASE } from './attractorApi'

export async function fetchLlmProfiles(): Promise<LlmProfileMetadata[]> {
    let response: Response
    try {
        response = await fetch(`${ATTRACTOR_API_BASE}/llm-profiles`)
    } catch {
        return []
    }
    if (!response.ok) {
        return []
    }
    const payload = await response.json()
    if (!payload || typeof payload !== 'object' || !Array.isArray(payload.profiles)) {
        return []
    }
    const profiles = payload.profiles as unknown[]
    return profiles
        .filter((entry: unknown): entry is Record<string, unknown> => Boolean(entry) && typeof entry === 'object')
        .map((entry: Record<string, unknown>): LlmProfileMetadata => ({
            id: typeof entry.id === 'string' ? entry.id : '',
            label: typeof entry.label === 'string' ? entry.label : null,
            provider: typeof entry.provider === 'string' ? entry.provider : '',
            models: Array.isArray(entry.models) ? entry.models.filter((model: unknown): model is string => typeof model === 'string') : [],
            default_model: typeof entry.default_model === 'string' ? entry.default_model : null,
            configured: entry.configured === true,
        }))
        .filter((profile: LlmProfileMetadata) => profile.id && profile.provider && profile.models.length > 0)
}
