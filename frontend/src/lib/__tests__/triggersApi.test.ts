import { ApiSchemaError, parseTriggerResponse } from '@/lib/workspaceClient'
import { describe, expect, it } from 'vitest'

const baseTriggerResponse = {
  id: 'trigger-1',
  name: 'Example trigger',
  enabled: true,
  protected: false,
  revision: 'revision-1',
  source_type: 'schedule',
  created_at: '2026-03-22T00:00:00Z',
  updated_at: '2026-03-22T00:00:00Z',
  action: {
    flow_name: 'example.dot',
    project_path: null,
    static_context: {},
  },
  source: {
    kind: 'interval',
    interval_seconds: 300,
  },
  state: {
    last_fired_at: null,
    last_result: null,
    last_error: null,
    next_run_at: null,
    recent_history: [],
  },
}

describe('parseTriggerResponse', () => {
  it('accepts supported trigger source types', () => {
    expect(parseTriggerResponse(baseTriggerResponse).source_type).toBe('schedule')
    expect(parseTriggerResponse({ ...baseTriggerResponse, source_type: 'poll', source: { url: 'https://example.com' } }).source_type).toBe('poll')
    expect(parseTriggerResponse({ ...baseTriggerResponse, source_type: 'webhook', source: { webhook_key: 'secret-key' } }).source_type).toBe('webhook')
    expect(parseTriggerResponse({ ...baseTriggerResponse, source_type: 'flow_event', source: { flow_name: 'observed.dot', statuses: ['completed'] } }).source_type).toBe('flow_event')
  })

  it('rejects unsupported trigger source types', () => {
    expect(() => parseTriggerResponse({
      ...baseTriggerResponse,
      revision: 'revision-1',
      source_type: 'unsupported_source',
      source: {},
    })).toThrow(ApiSchemaError)
  })
})
