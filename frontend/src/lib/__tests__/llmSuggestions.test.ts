import { effortNames, sessionControls } from '@/lib/llmSuggestions'
import { modelSettingsForApi, parseModelSettingsView } from '@/lib/api/settingsApi'
import { describe, expect, it } from 'vitest'

describe('llm suggestions', () => {
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
})
