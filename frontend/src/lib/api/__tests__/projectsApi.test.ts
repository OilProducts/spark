import {
  parseProjectBrowseResponse,
  parseProjectChatModelsResponse,
} from '@/lib/api/projectsApi'

describe('projectsApi parsing', () => {
  it('parses project browse roots', () => {
    const payload = parseProjectBrowseResponse({
      current_path: '/projects',
      parent_path: '/',
      roots: ['/projects', '/workspace'],
      entries: [
        { name: 'app', path: '/projects/app', is_dir: true },
      ],
    })

    expect(payload).toEqual({
      current_path: '/projects',
      parent_path: '/',
      roots: ['/projects', '/workspace'],
      entries: [
        { name: 'app', path: '/projects/app', is_dir: true },
      ],
    })
  })

  it('parses project chat model metadata', () => {
    const payload = parseProjectChatModelsResponse({
      providers: {
        codex: { status: 'available', error: null },
        anthropic: { status: 'unavailable', error: 'HTTP 401' },
      },
      provider_reasoning_efforts: { openai: ['none', 'minimal', 'high', 42] },
      models: [
        {
          id: 'gpt-5.4',
          display: 'GPT-5.4',
          is_default: true,
          supported_reasoning_efforts: ['none', 'minimal', 'low', 'medium', 'high', 'xhigh', 'max', 'ultra', 'unknown'],
          default_reasoning_effort: 'medium',
        },
      ],
    })

    expect(payload.models).toEqual([
      {
        provider: 'codex',
        id: 'gpt-5.4',
        display: 'GPT-5.4',
        is_default: true,
        supported_reasoning_efforts: ['none', 'minimal', 'low', 'medium', 'high', 'xhigh', 'max', 'ultra', 'unknown'],
        default_reasoning_effort: 'medium',
      },
    ])
    expect(payload.provider_reasoning_efforts).toEqual({ openai: ['none', 'minimal', 'high'] })
    expect(payload.providers.codex).toEqual({ status: 'available', error: null })
    expect(payload.providers.anthropic).toEqual({ status: 'unavailable', error: 'HTTP 401' })
  })

  it('keeps an ACP harness status, its capability reasons and raw config options', () => {
    const options = [
      { id: 'Mode/ID', name: 'Approval', category: 'mode', type: 'select', currentValue: 'Ask',
        options: [{ group: 'g', name: 'Manual', options: [{ value: 'Ask', name: 'Ask first' }] }] },
      { id: 'dial', name: 'Dial', type: 'slider', currentValue: 3 },
    ]
    const capability = { state: 'unsupported', reason: 'No systemPrompt.' }
    const payload = parseProjectChatModelsResponse({
      models: [],
      providers: {
        codex: { status: 'available', error: null, label: 'Codex' },
        'acp:fake': { status: 'available', error: null, label: 'Fake ACP', setup_status: 'ready',
          capabilities: { pinned_instructions: capability }, chat: capability, flow: { state: 'supported', reason: 'Ready.' },
          config_options: options, modes: { currentModeId: 'code', availableModes: [{ id: 'code', name: 'Code' }] } },
        'acp:locked': { status: 'unavailable', error: 'Authentication required', setup_status: 'unauthenticated' },
      },
    })

    expect(payload.providers.codex).toEqual({ status: 'available', error: null, label: 'Codex' })
    expect(payload.providers['acp:fake']).toEqual({ status: 'available', error: null, label: 'Fake ACP', setup_status: 'ready',
      capabilities: { pinned_instructions: capability }, chat: capability, flow: { state: 'supported', reason: 'Ready.' },
      config_options: options, modes: { currentModeId: 'code', availableModes: [{ id: 'code', name: 'Code' }] } })
    expect(payload.providers['acp:locked'].setup_status).toBe('unauthenticated')
    expect(() => parseProjectChatModelsResponse({ models: [], providers: {
      codex: { status: 'available', error: null }, 'acp:x': { status: 'available', error: null, chat: { state: 'maybe', reason: '' } },
    } })).toThrow()
  })
})
