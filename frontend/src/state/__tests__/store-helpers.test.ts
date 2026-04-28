import { DEFAULT_UI_DEFAULTS, UI_DEFAULTS_STORAGE_KEY, loadUiDefaults } from '@/state/store-helpers'
import { describe, expect, it } from 'vitest'

describe('store helpers', () => {
  it('loads gpt-5.5 as the default UI model when no saved defaults exist', () => {
    localStorage.removeItem(UI_DEFAULTS_STORAGE_KEY)

    expect(loadUiDefaults()).toEqual(DEFAULT_UI_DEFAULTS)
    expect(loadUiDefaults()).toEqual({
      llm_model: 'gpt-5.5',
      llm_provider: '',
      llm_profile: '',
      reasoning_effort: '',
    })
  })

  it('preserves saved UI defaults from local storage', () => {
    localStorage.setItem(UI_DEFAULTS_STORAGE_KEY, JSON.stringify({
      llm_model: 'claude-sonnet-4-6',
      llm_provider: 'anthropic',
      llm_profile: '',
      reasoning_effort: 'high',
    }))

    expect(loadUiDefaults()).toEqual({
      llm_model: 'claude-sonnet-4-6',
      llm_provider: 'anthropic',
      llm_profile: '',
      reasoning_effort: 'high',
    })
  })
})
