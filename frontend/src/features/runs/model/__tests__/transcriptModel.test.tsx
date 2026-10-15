import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { useRunTranscriptStore } from '../../state/runTranscriptStore'

import type { RunTranscriptSegment } from '@/lib/api/attractorApi'
import { buildRunTranscriptRow } from '../transcriptModel'
import { RunTranscript } from '../../components/RunTranscriptGroups'

const segment = (overrides: Partial<RunTranscriptSegment>): RunTranscriptSegment => ({
  id: 'segment-1',
  turn_id: 'root:implement:attempt-0',
  order: 1,
  kind: 'assistant_message',
  role: 'assistant',
  status: 'complete',
  timestamp: '2026-07-08T10:00:00Z',
  updated_at: '2026-07-08T10:00:00Z',
  content: 'Done.',
  completed_at: null,
  error: null,
  artifact_id: null,
  phase: null,
  tool_call: null,
  request_user_input: null,
  source: null,
  node_id: 'implement',
  stage_index: 0,
  attempt: 0,
  latest_sequence: 1,
  source_scope: 'root',
  source_flow_name: null,
  source_parent_node_id: null,
  source_run_id: null,
  ...overrides,
})

describe('buildRunTranscriptRow', () => {
  it('maps messages, reasoning, tool calls, compaction and warnings onto the shared rows and skips other kinds', () => {
    const rows = [
      segment({ id: 's-answer' }),
      segment({ id: 's-thinking', kind: 'reasoning', content: '**Weighing options** details here' }),
      segment({
        id: 's-tool',
        kind: 'tool_call',
        role: 'system',
        tool_call: {
          id: 'call-1',
          kind: 'command_execution',
          status: 'completed',
          title: 'ls -la',
          command: 'ls -la',
          output: 'files',
          output_size: null,
          output_truncated: false,
          file_paths: [],
        },
      }),
      segment({ id: 's-compaction', kind: 'context_compaction', role: 'system', content: '' }),
      segment({ id: 's-warning', kind: 'agent_event', category: 'warning', role: 'system', content: 'API error 529; retrying' }),
      segment({ id: 's-session', kind: 'agent_event', category: 'lifecycle', role: 'system', content: 'session started' }),
      segment({ id: 's-question', kind: 'request_user_input', role: 'system' }),
    ].map(buildRunTranscriptRow)
    expect(rows.map((row) => row?.kind ?? null)).toEqual(['message', 'thinking', 'tool_call', 'event', 'event', null, null])
    expect(rows.slice(3, 5)).toEqual([
      expect.objectContaining({ text: 'Context compacted.', warning: false }),
      expect.objectContaining({ text: 'API error 529; retrying', warning: true }),
    ])
  })

  it('renders compaction and warning rows as event lines', () => {
    const rows = [
      segment({ id: 's-compaction', kind: 'context_compaction', role: 'system', content: 'Context compacted (21.6k \u2192 1.9k tokens).' }),
      segment({ id: 's-warning', kind: 'agent_event', category: 'warning', role: 'system', content: 'Rate limited until 15:00 UTC' }),
    ].map((entry) => buildRunTranscriptRow(entry)!)
    render(<RunTranscript rows={rows} showThinking={false} />)
    expect(screen.getByTestId('run-event-row-s-compaction')).toHaveTextContent('Context compacted (21.6k \u2192 1.9k tokens).')
    expect(screen.getByTestId('run-event-row-s-warning')).toHaveTextContent('Rate limited until 15:00 UTC')
  })
})

describe('runTranscriptStore.applySegmentUpsert', () => {
  const contents = (runId: string) => (
    useRunTranscriptStore.getState().byRunId[runId].segments.map((entry) => entry.content)
  )

  it('upserts a later visit without overwriting the earlier one', () => {
    const shared = { turn_id: 'response', id: 'final-response', node_id: 'evaluate', source_run_id: 'run-1' }
    const store = useRunTranscriptStore.getState()
    store.setSegments('run-1', [segment({ ...shared, stage_index: 3, content: 'First.' })], 0)
    store.applySegmentUpsert('run-1', segment({ ...shared, stage_index: 7, content: 'Second, streaming.' }))
    store.applySegmentUpsert('run-1', segment({ ...shared, stage_index: 7, content: 'Second, done.' }))
    expect(contents('run-1')).toEqual(['First.', 'Second, done.'])
  })

  it('upserts a child run segment without overwriting the root one', () => {
    const shared = { turn_id: 'response', id: 'final-response', node_id: 'implement' }
    const store = useRunTranscriptStore.getState()
    store.setSegments('run-2', [segment({ ...shared, source_run_id: 'run-2', content: 'Root.' })], 0)
    store.applySegmentUpsert('run-2', segment({ ...shared, source_run_id: 'run-child', source_scope: 'child', content: 'Child.' }))
    expect(contents('run-2')).toEqual(['Root.', 'Child.'])
  })
})
