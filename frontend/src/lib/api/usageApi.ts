import { ApiSchemaError, expectObjectRecord, expectString } from './shared'
import { fetchWorkspaceJsonValidated } from './apiClient'

/** One plan limit window of the account a harness is signed in with. */
export interface UsageWindow {
    label: string
    /** 0 to 100. */
    used_percent: number
    resets_at: string | null
}

export type UsageHarness = 'claude' | 'codex' | 'antigravity'

export interface UsageResponse {
    windows: UsageWindow[]
    registered: boolean
    name?: string
}

function parseUsageResponse(payload: unknown, endpoint: string): UsageResponse {
    const record = expectObjectRecord(payload, endpoint)
    if (record.registered === false) {
        return { registered: false, windows: [] }
    }
    const windows = record.windows
    if (!Array.isArray(windows)) throw new ApiSchemaError(endpoint, 'Expected "windows" to be an array.')
    return {
        registered: true,
        ...(typeof record.name === 'string' && record.name.trim() ? { name: record.name } : {}),
        windows: windows.map((value) => {
            const window = expectObjectRecord(value, endpoint)
            if (typeof window.used_percent !== 'number') throw new ApiSchemaError(endpoint, 'Expected "used_percent" to be a number.')
            return {
                label: expectString(window.label, endpoint, 'label'),
                used_percent: window.used_percent,
                resets_at: window.resets_at === null ? null : expectString(window.resets_at, endpoint, 'resets_at'),
            }
        }),
    }
}

export function fetchUsage(harness: UsageHarness): Promise<UsageResponse> {
    return fetchWorkspaceJsonValidated(`/${harness}/usage`, { cache: 'no-store' }, `/${harness}/usage`, parseUsageResponse)
}
