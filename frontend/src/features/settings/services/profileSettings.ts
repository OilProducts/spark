import { fetchWorkspaceJsonValidated } from '@/lib/api/apiClient'
import { parseRepairableStored, parseSettingsFeedback, ApiSchemaError, expectObjectRecord, expectString } from '@/lib/api/shared'

export interface LlmProfileSettings {
    id: string
    provider: string
    base_url: string
    label?: string
    api_key_env?: string
    models: string[]
    default_model?: string
}
export interface ExecutionProfileSettings {
    id: string
    label: string
    mode: 'native' | 'local_container'
    enabled: boolean
    image?: string
    capabilities: string[]
    metadata: Record<string, unknown>
}
export interface ExecutionProfilesSettings {
    profiles: ExecutionProfileSettings[]
    default_execution_profile_id: string | null
}
export interface ProfileSettingsView<T> { revision: string; stored: T | null; repair_defaults?: T; validation_errors?: string[]; credential_status?: Record<string, string> }
export type ProfileSection = 'llm_profiles' | 'execution_profiles'

const strings = (value: unknown, endpoint: string): string[] => {
    if (!Array.isArray(value)) throw new ApiSchemaError(endpoint, 'Expected a list of strings.')
    return value.map((entry) => expectString(entry, endpoint, 'profile field'))
}
export function parseLlmProfiles(value: unknown, endpoint: string): LlmProfileSettings[] {
    if (!Array.isArray(value)) throw new ApiSchemaError(endpoint, 'Expected LLM profiles.')
    return value.map((entry) => {
        const profile = expectObjectRecord(entry, endpoint)
        return {
            id: expectString(profile.id, endpoint, 'id'), provider: expectString(profile.provider, endpoint, 'provider'),
            base_url: expectString(profile.base_url, endpoint, 'base_url'), models: strings(profile.models, endpoint),
            ...(profile.label != null ? { label: expectString(profile.label, endpoint, 'label') } : {}),
            ...(profile.api_key_env != null ? { api_key_env: expectString(profile.api_key_env, endpoint, 'api_key_env') } : {}),
            ...(profile.default_model != null ? { default_model: expectString(profile.default_model, endpoint, 'default_model') } : {}),
        }
    })
}
export function parseExecutionProfiles(value: unknown, endpoint: string): ExecutionProfilesSettings {
    const record = expectObjectRecord(value, endpoint)
    if (!Array.isArray(record.profiles)) throw new ApiSchemaError(endpoint, 'Expected execution profiles.')
    return {
        default_execution_profile_id: record.default_execution_profile_id == null ? null : expectString(record.default_execution_profile_id, endpoint, 'default_execution_profile_id'),
        profiles: record.profiles.map((entry) => {
            const profile = expectObjectRecord(entry, endpoint)
            if (profile.mode !== 'native' && profile.mode !== 'local_container') throw new ApiSchemaError(endpoint, 'Invalid execution mode.')
            if (typeof profile.enabled !== 'boolean') throw new ApiSchemaError(endpoint, 'Invalid enabled flag.')
            return { id: expectString(profile.id, endpoint, 'id'), label: expectString(profile.label, endpoint, 'label'), mode: profile.mode,
                enabled: profile.enabled, capabilities: strings(profile.capabilities, endpoint), metadata: expectObjectRecord(profile.metadata, endpoint),
                ...(profile.image != null ? { image: expectString(profile.image, endpoint, 'image') } : {}) }
        }),
    }
}
export function profileSettingsRequest<T>(section: ProfileSection, parse: (value: unknown, endpoint: string) => T, update?: { revision: string; value: T }): Promise<ProfileSettingsView<T>> {
    return fetchWorkspaceJsonValidated('/settings', update ? {
        method: 'PATCH', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ expected_revision: update.revision, section, value: update.value }),
    } : undefined, '/workspace/api/settings', (payload, endpoint) => {
        const view = expectObjectRecord(expectObjectRecord(payload, endpoint)[section], endpoint)
        const status = view.credential_status == null ? undefined : expectObjectRecord(view.credential_status, endpoint)
        return { revision: expectString(view.revision, endpoint, 'revision'), ...parseRepairableStored(view, (value) => parse(value, endpoint)), ...parseSettingsFeedback(view, endpoint),
            credential_status: status && Object.fromEntries(Object.entries(status).map(([key, value]) => [key, expectString(value, endpoint, 'credential_status')])) }
    })
}
