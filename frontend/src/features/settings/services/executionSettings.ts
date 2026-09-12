import { fetchWorkspaceJsonValidated } from '@/lib/api/apiClient'
import { parseRepairableStored, parseSettingsFeedback, ApiSchemaError, expectObjectRecord, expectString } from '@/lib/api/shared'

export const providers = ['openai', 'anthropic', 'gemini', 'openrouter', 'litellm', 'openai_compatible'] as const
export type Provider = typeof providers[number]
export interface ProviderConnection {
    base_url?: string | null
    api_key_env?: string | null
    organization?: string | null
    project?: string | null
    http_referer?: string | null
    title?: string | null
}
export type ProviderSettings = Partial<Record<Provider, ProviderConnection>>
export interface ProviderSettingsView {
    revision: string
    stored: ProviderSettings | null
    repair_defaults?: ProviderSettings
    effective: ProviderSettings | null
    sources?: Record<string, string>
    validation_errors?: string[]
    credential_status: Partial<Record<Provider, boolean>>
}
function parseProviders(payload: unknown, endpoint: string): ProviderSettingsView {
    const view = expectObjectRecord(expectObjectRecord(payload, endpoint).providers, endpoint)
    const parse = (value: unknown): ProviderSettings => {
        const result: ProviderSettings = {}
        for (const [name, connection] of Object.entries(expectObjectRecord(value, endpoint))) {
            if (!providers.includes(name as Provider)) throw new ApiSchemaError(endpoint, 'Unsupported provider.')
            const fields = expectObjectRecord(connection, endpoint)
            const parsed: ProviderConnection = {}
            for (const key of ['base_url', 'api_key_env', 'organization', 'project', 'http_referer', 'title'] as const) {
                if (fields[key] != null) parsed[key] = expectString(fields[key], endpoint, key)
            }
            result[name as Provider] = parsed
        }
        return result
    }
    const status = expectObjectRecord(view.credential_status ?? {}, endpoint)
    if (Object.values(status).some((value) => typeof value !== 'boolean')) throw new ApiSchemaError(endpoint, 'Invalid credential status.')
    return { revision: expectString(view.revision, endpoint, 'revision'), ...parseRepairableStored(view, parse), effective: view.effective == null ? null : parse(view.effective), ...parseSettingsFeedback(view, endpoint), credential_status: status as ProviderSettingsView['credential_status'] }
}
export function providerFieldError(key: keyof ProviderConnection, value: string | null | undefined): string {
    if (value == null || value === '') return ''
    if (key === 'api_key_env') return /^[A-Za-z_][A-Za-z0-9_]*$/.test(value) ? '' : 'Enter an environment-variable name.'
    if (key === 'base_url' || key === 'http_referer') {
        try {
            const url = new URL(value)
            if (['http:', 'https:'].includes(url.protocol) && !url.username && !url.password && !url.search && !url.hash) return ''
        } catch { /* Show the same error without reflecting a credential-bearing URL. */ }
        return 'Enter an HTTP(S) URL without credentials, query or fragment.'
    }
    return value.trim() && !/[\r\n\t]/.test(value) ? '' : 'Enter a nonempty single-line value.'
}
export function fetchProviderSettings(): Promise<ProviderSettingsView> {
    return fetchWorkspaceJsonValidated('/settings', undefined, '/workspace/api/settings', parseProviders)
}
export function saveProviderSettings(revision: string, value: ProviderSettings): Promise<ProviderSettingsView> {
    return fetchWorkspaceJsonValidated('/settings', { method: 'PATCH', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ expected_revision: revision, section: 'providers', value }) }, '/workspace/api/settings', parseProviders)
}

export interface NativeAgentSettings {
    codex_binary?: string | null
    codex_runtime_root?: string | null
    codex_seed_dir?: string | null
    claude_binary?: string | null
    claude_config_dir?: string | null
    claude_permission_mode?: string | null
    codex_jsonrpc_trace?: boolean | null
    agent_trace?: boolean | null
}
export interface AgentSettings {
    native?: NativeAgentSettings
    environment_inheritance?: 'inherit_all' | 'inherit_none' | 'inherit_core_only'

    max_turns: number
    max_tool_rounds_per_input: number
    default_command_timeout_ms: number
    max_command_timeout_ms: number
    tool_output_limits: Record<string, number>
    line_limits: Record<string, number>
    enable_loop_detection: boolean
    loop_detection_window: number
    max_subagent_depth: number
}
export interface AgentSettingsView { revision: string; stored: AgentSettings | null; repair_defaults?: AgentSettings; effective: AgentSettings | null; active_startup?: NativeAgentSettings; sources?: Record<string, string>; validation_errors?: string[] }
function parseAgents(payload: unknown, endpoint: string): AgentSettingsView {
    const view = expectObjectRecord(expectObjectRecord(payload, endpoint).agents, endpoint)
    const parse = (value: unknown): AgentSettings => {
        const fields = expectObjectRecord(value, endpoint)
        for (const key of ['max_turns', 'max_tool_rounds_per_input', 'default_command_timeout_ms', 'max_command_timeout_ms', 'loop_detection_window', 'max_subagent_depth']) {
            if (!Number.isSafeInteger(fields[key]) || (fields[key] as number) < 0) throw new ApiSchemaError(endpoint, `Invalid ${key}.`)
        }
        for (const key of ['tool_output_limits', 'line_limits']) {
            if (Object.values(expectObjectRecord(fields[key], endpoint)).some((value) => !Number.isSafeInteger(value) || (value as number) < 0)) throw new ApiSchemaError(endpoint, `Invalid ${key}.`)
        }
        if (typeof fields.enable_loop_detection !== 'boolean') throw new ApiSchemaError(endpoint, 'Invalid loop detection.')
        if (fields.native != null) {
            const native = expectObjectRecord(fields.native, endpoint)
            for (const key of ['codex_binary', 'codex_runtime_root', 'codex_seed_dir', 'claude_binary', 'claude_config_dir', 'claude_permission_mode']) {
                if (native[key] != null && typeof native[key] !== 'string') throw new ApiSchemaError(endpoint, `Invalid ${key}.`)
            }
            for (const key of ['codex_jsonrpc_trace', 'agent_trace']) {
                if (native[key] != null && typeof native[key] !== 'boolean') throw new ApiSchemaError(endpoint, `Invalid ${key}.`)
            }
        }
        if (fields.environment_inheritance != null && !['inherit_all', 'inherit_none', 'inherit_core_only'].includes(String(fields.environment_inheritance))) throw new ApiSchemaError(endpoint, 'Invalid environment inheritance.')
        return fields as unknown as AgentSettings
    }
    const startup = view.active_startup == null ? undefined : expectObjectRecord(view.active_startup, endpoint)
    const active_startup = startup && Object.fromEntries(['codex_runtime_root', 'codex_seed_dir', 'claude_config_dir']
        .map((key) => [key, startup[key] == null ? null : expectString(startup[key], endpoint, key)]))
    return { revision: expectString(view.revision, endpoint, 'revision'), ...parseRepairableStored(view, parse), active_startup, effective: view.effective == null ? null : parse(view.effective), ...parseSettingsFeedback(view, endpoint) }
}
export function fetchAgentSettings(): Promise<AgentSettingsView> {
    return fetchWorkspaceJsonValidated('/settings', undefined, '/workspace/api/settings', parseAgents)
}
export function saveAgentSettings(revision: string, value: AgentSettings): Promise<AgentSettingsView> {
    return fetchWorkspaceJsonValidated('/settings', { method: 'PATCH', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ expected_revision: revision, section: 'agents', value }) }, '/workspace/api/settings', parseAgents)
}
