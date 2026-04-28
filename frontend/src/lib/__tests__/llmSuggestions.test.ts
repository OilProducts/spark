import { getLlmSelectionOptions, getModelSuggestions, LLM_PROVIDER_OPTIONS, splitLlmSelection } from '@/lib/llmSuggestions'
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
      'openai',
      'anthropic',
      'gemini',
      'openrouter',
      'litellm',
      'openai_compatible',
    ])
    expect(getModelSuggestions('openrouter')[0]).toBe('openai/gpt-5.4')
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
