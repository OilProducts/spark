import { effortNames, getLlmSelectionOptions, getModelSuggestions, LLM_PROVIDER_OPTIONS, providerLabel, sessionControls, splitLlmSelection } from '@/lib/llmSuggestions'
import { modelSettingsForApi, parseModelSettingsView } from '@/lib/api/settingsApi'
import { describe, expect, it } from 'vitest'

describe('llm suggestions', () => {
  it('surfaces gpt-5.5 first for OpenAI model suggestions', () => {
    expect(getModelSuggestions('openai').slice(0, 3)).toEqual([
      'gpt-5.5',
      'gpt-5.4',
      'gpt-5.2',
    ])
  })

  it('includes OpenRouter and LiteLLM providers without forcing a LiteLLM model default', () => {
    expect(LLM_PROVIDER_OPTIONS).toEqual([
      'codex',
      'claude-code',
      'openai',
      'anthropic',
      'gemini',
      'openrouter',
      'litellm',
      'openai_compatible',
    ])
    expect(getModelSuggestions('openrouter')[0]).toBe('openai/gpt-5.4')
    expect(getModelSuggestions('claude-code')).toEqual([])
    expect(getModelSuggestions('litellm')).toEqual([])
    expect(getModelSuggestions('openai_compatible')).toEqual([])
  })

  it('uses configured profile models for profile-backed model suggestions', () => {
    const profiles = [
      {
        id: 'lan-lmstudio',
        label: 'LAN LM Studio',
        provider: 'openai_compatible',
        models: ['qwen2.5-coder-32b-instruct', 'deepseek-coder-v2'],
        default_model: 'qwen2.5-coder-32b-instruct',
        configured: true,
      },
    ]

    expect(getLlmSelectionOptions(profiles)).toContain('lan-lmstudio')
    expect(getModelSuggestions('lan-lmstudio', profiles)).toEqual([
      'qwen2.5-coder-32b-instruct',
      'deepseek-coder-v2',
    ])
  })

  it('adds discovered providers with their reported labels and no static models', () => {
    expect(getLlmSelectionOptions([], ['acp:fake', 'codex'])).toEqual([...LLM_PROVIDER_OPTIONS, 'acp:fake'])
    expect(getModelSuggestions('acp:fake')).toEqual([])
    const statuses = { 'acp:fake': { status: 'available' as const, error: null, label: 'Fake ACP' } }
    expect(providerLabel('acp:fake', statuses)).toBe('Fake ACP')
    expect(providerLabel('openai', statuses)).toBe('openai')
  })

  it('offers select options beyond model and thought level as session controls, by exact ID', () => {
    const status = { status: 'available' as const, error: null, config_options: [
      { id: 'model', name: 'Model', category: 'model', type: 'select', currentValue: 'a', options: [{ value: 'a', name: 'A' }] },
      { id: 'effort', name: 'Effort', category: 'thought_level', type: 'select', currentValue: 'Lo', options: [{ value: 'Lo', name: 'Low' }, { value: 'Hi/2', name: 'High' }] },
      { id: 'Mode/ID', name: 'Approval', category: 'mode', type: 'select', currentValue: 'Ask',
        options: [{ group: 'manual', name: 'Manual', options: [{ value: 'Ask', name: 'Ask first' }] }, { group: 'auto', name: 'Automatic', options: [{ value: 'Auto/Edit', name: 'Auto edit' }] }] },
      { id: 'second-model', name: 'Fallback model', category: 'model', type: 'select', currentValue: 'b', options: [{ value: 'b', name: 'B' }] },
      { id: 'verbose', name: 'Verbose', type: 'boolean', currentValue: false },
      { id: 'dial', name: 'Dial', type: 'slider', currentValue: 3 },
    ] }
    expect(sessionControls(status)).toEqual([
      { id: 'Mode/ID', name: 'Approval', description: undefined, values: [
        { value: 'Ask', name: 'Ask first', group: 'Manual' }, { value: 'Auto/Edit', name: 'Auto edit', group: 'Automatic' }] },
      { id: 'second-model', name: 'Fallback model', description: undefined, values: [{ value: 'b', name: 'B' }] },
    ])
    expect(effortNames(status)).toEqual({ Lo: 'Low', 'Hi/2': 'High' })
    expect(sessionControls({ status: 'available', error: null })).toEqual([])
  })

  it('carries session controls through model settings, with none sent as a reset', () => {
    const view = (effective: Record<string, unknown>) => parseModelSettingsView({ models: {
      scope: 'conversation', source: 'conversation', revision: '1', stored: null, effective,
    } }, '/settings').effective
    const base = { provider: 'acp:fake', llm_profile: null, model: null, reasoning_effort: null }
    expect(view({ ...base, session_config: { 'Mode/ID': 'Auto/Edit' } })?.session_config).toEqual({ 'Mode/ID': 'Auto/Edit' })
    expect(view(base)).not.toHaveProperty('session_config')
    expect(() => view({ ...base, session_config: { mode: 3 } })).toThrow()
    expect(modelSettingsForApi({ provider: null, llm_profile: null, model: null, reasoning_effort: null, session_config: {} })).toBeNull()
  })

  it('splits profile selections away from provider selections', () => {
    const profiles = [
      {
        id: 'lan-lmstudio',
        label: 'LAN LM Studio',
        provider: 'openai_compatible',
        models: ['qwen2.5-coder-32b-instruct'],
        default_model: 'qwen2.5-coder-32b-instruct',
        configured: true,
      },
    ]

    expect(splitLlmSelection('lan-lmstudio', profiles)).toEqual({
      llm_provider: '',
      llm_profile: 'lan-lmstudio',
    })
    expect(splitLlmSelection('openai_compatible', profiles)).toEqual({
      llm_provider: 'openai_compatible',
      llm_profile: '',
    })
  })
})
