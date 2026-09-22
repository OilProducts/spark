import { fetchWorkspaceJsonValidated } from '@/lib/api/apiClient'
import { ApiSchemaError, expectObjectRecord, expectString } from '@/lib/api/shared'
import type { Appearance } from '@/lib/theme'

const ID_KEY = 'spark.client_id'
const validId = (value: string) => /^[A-Za-z0-9_-]{1,80}$/.test(value)

export async function clientIdentity(): Promise<string> {
    const invoke = window.__TAURI__?.core?.invoke
    if (invoke) {
        const id = await invoke<string>('desktop_client_identity')
        if (typeof id !== 'string' || !validId(id)) throw new Error('Invalid Desktop client identity.')
        return id
    }
    const stored = localStorage.getItem(ID_KEY)
    if (stored !== null) {
        if (!validId(stored)) throw new Error('Invalid browser client identity in spark.client_id.')
        return stored
    }
    const random = Array.from(crypto.getRandomValues(new Uint8Array(16)), (byte) => byte.toString(16).padStart(2, '0')).join('')
    const id = `browser-${random}`
    localStorage.setItem(ID_KEY, id)
    return id
}

export interface LayoutEdgePorts {
    source_side: 'top' | 'right' | 'bottom' | 'left'
    target_side: 'top' | 'right' | 'bottom' | 'left'
    source_slot: number
    target_slot: number
}
export type FlowEdgePorts = Record<string, Record<string, LayoutEdgePorts>>
export type FlowNodePositions = Record<string, Record<string, { x: number; y: number }>>
export interface RunPresentation {
    sort?: 'newest' | 'oldest' | null
    activity_mode?: 'all' | 'transcript' | 'events' | null
    inspector_tab?: 'activity' | 'result' | 'details' | 'context' | 'artifacts' | null
    timeline_category?: 'all' | import('@/features/runs/model/shared').TimelineEventCategory | null
    timeline_severity?: 'all' | import('@/features/runs/model/shared').TimelineSeverity | null
    graph_height?: number | null
}
export const runPresentationChoices = {
    sort: ['newest', 'oldest'], activity_mode: ['all', 'transcript', 'events'],
    inspector_tab: ['activity', 'result', 'details', 'context', 'artifacts'],
    timeline_category: ['all', 'lifecycle', 'stage', 'parallel', 'interview', 'checkpoint', 'log', 'runtime', 'state', 'metadata', 'other'],
    timeline_severity: ['all', 'info', 'warning', 'error'],
} as const
export interface ClientPreferences {
    run_presentation?: RunPresentation | null
    flow_node_positions?: FlowNodePositions | null
    flow_edge_ports?: FlowEdgePorts | null
    editor_mode: 'structured' | 'raw' | null
    editor_sidebar_width: number | null
    home_sidebar_primary_split_ratio?: number | null
    show_advanced_controls?: boolean | null
    expand_child_flows?: boolean | null
    graph_settings_open?: boolean | null
    runs_scope?: 'active' | 'all' | null
    triggers_scope?: 'active' | 'all' | null
    appearance?: Appearance | null
}
export interface ClientPreferencesView {
    client_id: string
    browser_migration_version?: number
    revision: string
    stored: ClientPreferences
    effective: ClientPreferences & { editor_mode: 'structured' | 'raw'; editor_sidebar_width: number }
}
export function parseClientPreferences(payload: unknown, endpoint: string): ClientPreferencesView {
    const record = expectObjectRecord(expectObjectRecord(payload, endpoint).preferences, endpoint)
    if (record.scope !== 'client') throw new ApiSchemaError(endpoint, 'Expected client scope.')
    if (record.browser_migration_version != null && record.browser_migration_version !== 0 && record.browser_migration_version !== 1) throw new ApiSchemaError(endpoint, 'Unsupported browser preference migration version.')
    const preferences = (value: unknown): ClientPreferences => {
        const fields = expectObjectRecord(value, endpoint)
        const mode = fields.editor_mode
        const width = fields.editor_sidebar_width
        if (mode !== null && mode !== 'structured' && mode !== 'raw') throw new ApiSchemaError(endpoint, 'Invalid editor mode.')
        if (width !== null && (typeof width !== 'number' || !Number.isInteger(width) || width < 256 || width > 560)) throw new ApiSchemaError(endpoint, 'Invalid sidebar width.')
        const presentation: Partial<ClientPreferences> = {}
        if (fields.run_presentation != null) {
            const run = expectObjectRecord(fields.run_presentation, endpoint)
            for (const [key, choices] of Object.entries(runPresentationChoices)) {
                if (run[key] != null && !(choices as readonly unknown[]).includes(run[key])) throw new ApiSchemaError(endpoint, `Invalid run ${key}.`)
            }
            if (run.graph_height != null && (typeof run.graph_height !== 'number' || !Number.isInteger(run.graph_height) || run.graph_height < 280 || run.graph_height > 960)) throw new ApiSchemaError(endpoint, 'Invalid run graph height.')
            presentation.run_presentation = run as RunPresentation
        }
        if (fields.flow_edge_ports != null) {
            const layouts = expectObjectRecord(fields.flow_edge_ports, endpoint)
            for (const edges of Object.values(layouts)) {
                for (const entry of Object.values(expectObjectRecord(edges, endpoint))) {
                    const ports = expectObjectRecord(entry, endpoint)
                    if (![ports.source_side, ports.target_side].every((side) => ['top', 'right', 'bottom', 'left'].includes(String(side))) || ![ports.source_slot, ports.target_slot].every((slot) => typeof slot === 'number' && Number.isInteger(slot) && slot >= 0 && slot <= 4294967295)) throw new ApiSchemaError(endpoint, 'Invalid edge ports.')
                }
            }
            presentation.flow_edge_ports = layouts as FlowEdgePorts
        }
        if (fields.flow_node_positions != null) {
            const layouts = expectObjectRecord(fields.flow_node_positions, endpoint)
            for (const nodes of Object.values(layouts)) {
                for (const position of Object.values(expectObjectRecord(nodes, endpoint))) {
                    const point = expectObjectRecord(position, endpoint)
                    if (typeof point.x !== 'number' || !Number.isFinite(point.x) || typeof point.y !== 'number' || !Number.isFinite(point.y)) throw new ApiSchemaError(endpoint, 'Invalid node position.')
                }
            }
            presentation.flow_node_positions = layouts as FlowNodePositions
        }
        const ratio = fields.home_sidebar_primary_split_ratio
        if (ratio != null && (typeof ratio !== 'number' || !Number.isFinite(ratio) || ratio < 0 || ratio > 1)) throw new ApiSchemaError(endpoint, 'Invalid home sidebar split.')
        if (ratio !== undefined) presentation.home_sidebar_primary_split_ratio = ratio as number | null
        for (const key of ['show_advanced_controls', 'expand_child_flows', 'graph_settings_open'] as const) {
            const field = fields[key]
            if (field != null && typeof field !== 'boolean') throw new ApiSchemaError(endpoint, `Invalid ${key}.`)
            if (field !== undefined) presentation[key] = field as boolean | null
        }
        for (const key of ['runs_scope', 'triggers_scope'] as const) {
            const field = fields[key]
            if (field != null && field !== 'active' && field !== 'all') throw new ApiSchemaError(endpoint, `Invalid ${key}.`)
            if (field !== undefined) presentation[key] = field as 'active' | 'all' | null
        }
        if (fields.appearance != null && fields.appearance !== 'system' && fields.appearance !== 'light' && fields.appearance !== 'dark') throw new ApiSchemaError(endpoint, 'Invalid appearance.')
        if (fields.appearance !== undefined) presentation.appearance = fields.appearance as Appearance | null
        return { editor_mode: mode, editor_sidebar_width: width, ...presentation }
    }
    const effective = preferences(record.effective)
    if (effective.editor_mode === null || effective.editor_sidebar_width === null) throw new ApiSchemaError(endpoint, 'Missing effective preferences.')
    return { browser_migration_version: typeof record.browser_migration_version === 'number' ? record.browser_migration_version : 0, client_id: expectString(record.client_id, endpoint, 'client_id'), revision: expectString(record.revision, endpoint, 'revision'),
        stored: preferences(record.stored), effective: { ...effective, editor_mode: effective.editor_mode, editor_sidebar_width: effective.editor_sidebar_width } }
}
export async function fetchClientPreferences(): Promise<ClientPreferencesView> {
    const id = await clientIdentity()
    return fetchWorkspaceJsonValidated(`/settings?client_id=${encodeURIComponent(id)}`, undefined, '/workspace/api/settings', parseClientPreferences)
}
export async function saveClientPreferences(view: ClientPreferencesView, preferences: ClientPreferences): Promise<ClientPreferencesView> {
    const result = await fetchWorkspaceJsonValidated('/settings', {
        method: 'PATCH', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ expected_revision: view.revision, section: 'client_preferences', value: { client_id: view.client_id, preferences } }),
    }, '/workspace/api/settings', parseClientPreferences)
    window.dispatchEvent(new Event('spark:settings-live-event'))
    return result
}

export function completePreferenceInteraction(patch: Partial<ClientPreferences>) {
    window.dispatchEvent(new CustomEvent('spark:preferences-completed', { detail: patch }))
}
