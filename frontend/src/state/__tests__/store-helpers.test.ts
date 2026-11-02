import { DEFAULT_UI_DEFAULTS } from '@/state/store-helpers'
import { describe, expect, it } from 'vitest'

describe('workspace model defaults', () => {
    it('uses provider defaults while loading', () => {
        expect(DEFAULT_UI_DEFAULTS).toEqual({ llm_provider: 'codex', llm_profile: '', llm_model: '', reasoning_effort: '' })
    })
})
