import { DEFAULT_UI_DEFAULTS } from '@/state/store-helpers'
import { loadAndMigrateModelDefaults } from '@/features/settings/services/modelDefaultsMigration'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const legacy = 'spark.ui_defaults'
const group = { provider: 'codex', llm_profile: null, model: null, reasoning_effort: null }
const response = (stored: unknown, effective = group) => Response.json({ models: { scope: 'workspace', source: 'workspace', revision: 'core-1', stored, effective } })

beforeEach(() => localStorage.clear())
afterEach(() => { vi.unstubAllGlobals(); localStorage.clear() })

describe('workspace model defaults migration', () => {
    it('uses provider defaults while loading and never reads browser defaults into the live store', () => {
        expect(DEFAULT_UI_DEFAULTS).toEqual({ llm_provider: 'codex', llm_profile: '', llm_model: '', reasoning_effort: '' })
    })
    it('backs up accessible legacy values and imports the group once', async () => {
        const old = JSON.stringify({ llm_provider: 'codex', llm_model: 'legacy', reasoning_effort: 'high' })
        localStorage.setItem(legacy, old)
        const fetch = vi.fn().mockResolvedValueOnce(response(null)).mockResolvedValueOnce(response({ ...group, model: 'legacy' }))
        vi.stubGlobal('fetch', fetch)
        await loadAndMigrateModelDefaults()
        expect(JSON.parse(String(fetch.mock.calls[1][1].body))).toEqual({ expected_revision: 'core-1', section: 'import_models', value: { ...group, model: 'legacy', reasoning_effort: 'high' } })
        expect(localStorage.getItem(`${legacy}.v0.bak`)).toBe(old)
        expect(localStorage.getItem(legacy)).toBeNull()
        fetch.mockResolvedValue(response(group))
        await loadAndMigrateModelDefaults()
        expect(fetch).toHaveBeenCalledTimes(3)
    })
    it('never overwrites an authoritative workspace choice with a later browser', async () => {
        localStorage.setItem(legacy, JSON.stringify({ llm_provider: 'anthropic', llm_model: 'old' }))
        const fetch = vi.fn().mockResolvedValue(response(group))
        vi.stubGlobal('fetch', fetch)
        const view = await loadAndMigrateModelDefaults()
        expect(view.effective).toEqual(group)
        expect(fetch).toHaveBeenCalledTimes(1)
        expect(localStorage.getItem(`${legacy}.v0.bak`)).not.toBeNull()
    })
    it('retains original browser data and its backup after a failed import, allowing retry', async () => {
        localStorage.setItem(legacy, JSON.stringify({ llm_provider: 'codex' }))
        const fetch = vi.fn().mockResolvedValueOnce(response(null)).mockResolvedValueOnce(Response.json({ detail: 'Invalid selection.' }, { status: 400 }))
        vi.stubGlobal('fetch', fetch)
        await expect(loadAndMigrateModelDefaults()).rejects.toThrow('Invalid selection.')
        expect(localStorage.getItem(legacy)).toBe(localStorage.getItem(`${legacy}.v0.bak`))
        fetch.mockResolvedValueOnce(response(null)).mockResolvedValueOnce(response(group))
        await loadAndMigrateModelDefaults()
        expect(localStorage.getItem(legacy)).toBeNull()
    })
})
