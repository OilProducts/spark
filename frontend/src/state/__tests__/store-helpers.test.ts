import { DEFAULT_UI_DEFAULTS, deriveGraphAttrErrors, normalizeGraphAttrs } from '@/state/store-helpers'
import { describe, expect, it } from 'vitest'

describe('normalizeGraphAttrs', () => {
    it('keeps unknown metadata values as supplied, without touching the input', () => {
        const unknown = {
            launcher: { enabled: true, owner: null, targets: [{ port: 80 }, null, ['a']] },
            list: [1, 'two', { three: false }],
            flag: false,
            count: 0,
            text: ' padded ',
            nothing: null,
        }
        const supplied = Object.freeze({ ...unknown })
        const snapshot = JSON.stringify(supplied)

        expect(normalizeGraphAttrs(supplied)).toEqual(unknown)
        expect(JSON.stringify(supplied)).toBe(snapshot)
    })

    it('drops undefined values', () => {
        expect(normalizeGraphAttrs({ extra: undefined })).toEqual({})
    })

    it('still normalizes recognized settings to trimmed strings and omits their nulls', () => {
        const normalized = normalizeGraphAttrs({
            title: '  Title  ',
            max_retries: 3,
            thinking_budget_tokens: 500 as unknown as string,
            goal: null as unknown as string,
            llm_model: undefined,
        })
        expect(normalized).toEqual({ title: 'Title', max_retries: '3', thinking_budget_tokens: '500' })
    })

    it('still validates recognized settings beside structured metadata', () => {
        const normalized = normalizeGraphAttrs({ max_retries: ' -1 ', launcher: { enabled: true } })
        expect(normalized.max_retries).toBe('-1')
        expect(deriveGraphAttrErrors(normalized)).toEqual({
            max_retries: 'Max retries default must be a non-negative integer.',
        })
    })
})

describe('workspace model defaults', () => {
    it('uses provider defaults while loading', () => {
        expect(DEFAULT_UI_DEFAULTS).toEqual({ llm_provider: 'codex', llm_profile: '', llm_model: '', reasoning_effort: '' })
    })
})
