import { fetchModelSettings, parseModelSettingsView, type ModelSettingsView } from '@/lib/api/settingsApi'
import { ApiHttpError } from '@/lib/api/shared'
import { fetchWorkspaceJsonValidated } from '@/lib/api/apiClient'

const LEGACY_KEY = 'spark.ui_defaults'

/** Only accessible browser data can be imported; authored workspace defaults always win. */
export async function loadAndMigrateModelDefaults(): Promise<ModelSettingsView> {
    let view = await fetchModelSettings()
    let legacy: string | null
    try { legacy = localStorage.getItem(LEGACY_KEY) } catch { return view }
    if (legacy === null) return view
    const backupKey = `${LEGACY_KEY}.v0.bak`
    const backup = localStorage.getItem(backupKey)
    if (backup !== null && backup !== legacy) throw new Error('Model defaults migration backup differs; resolve the browser backup before retrying.')
    localStorage.setItem(backupKey, legacy)
    if (view.stored === null) {
        let value: Record<string, unknown>
        try {
            const parsed: unknown = JSON.parse(legacy)
            if (parsed === null || typeof parsed !== 'object' || Array.isArray(parsed)) throw new Error()
            value = parsed as Record<string, unknown>
        } catch { throw new Error('Invalid legacy browser model defaults; the original data is retained.') }
        const field = (key: string) => {
            const entry = value[key]
            if (entry == null) return null
            if (typeof entry !== 'string') throw new Error('Invalid legacy browser model defaults; expected text fields.')
            return entry.trim() || null
        }
        const profile = field('llm_profile')
        try {
        view = await fetchWorkspaceJsonValidated('/settings', {
            method: 'PATCH', headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ expected_revision: view.revision, section: 'import_models', value: {
                provider: profile ? null : field('llm_provider') || 'codex', llm_profile: profile,
                model: field('llm_model'), reasoning_effort: field('reasoning_effort'),
            } }),
        }, '/workspace/api/settings', parseModelSettingsView)
        } catch (error) {
            if (!(error instanceof ApiHttpError) || error.status !== 409) throw error
            view = await fetchModelSettings()
            if (view.stored === null) throw error
        }
    }
    localStorage.removeItem(LEGACY_KEY)
    return view
}
