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
      models: [
        {
          id: 'gpt-5.4',
          display: 'GPT-5.4',
          is_default: true,
          supported_reasoning_efforts: ['low', 'medium', 'high', 'xhigh', 'unknown'],
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
        supported_reasoning_efforts: ['low', 'medium', 'high', 'xhigh'],
        default_reasoning_effort: 'medium',
      },
    ])
  })
})
