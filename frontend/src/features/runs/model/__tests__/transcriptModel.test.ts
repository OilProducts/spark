import { describe, expect, it } from 'vitest'

import type { RunTranscriptSegment } from '@/lib/api/attractorApi'
import { buildRunTranscriptGroups, runTranscriptGroupLabel } from '../transcriptModel'

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
  attempt: 0,
  latest_sequence: 1,
  source_scope: 'root',
  source_flow_name: null,
  source_parent_node_id: null,
  source_run_id: null,
  ...overrides,
})

describe('buildRunTranscriptGroups', () => {
  it('groups segments by node attempt and orders rows within each group', () => {
    const groups = buildRunTranscriptGroups([
      segment({ id: 's-answer', order: 2, latest_sequence: 4 }),
      segment({
        id: 's-thinking',
        kind: 'reasoning',
        order: 1,
        content: '**Weighing options** details here',
        latest_sequence: 2,
      }),
      segment({
        id: 's-retry',
        turn_id: 'root:implement:attempt-1',
        attempt: 1,
        content: 'Second try.',
        latest_sequence: 9,
      }),
    ])
    expect(groups).toHaveLength(2)
    expect(groups[0].rows.map((row) => row.kind)).toEqual(['thinking', 'message'])
    expect(groups[0].latestSequence).toBe(4)
    expect(groups[1].attempt).toBe(1)
    expect(runTranscriptGroupLabel(groups[1])).toBe('implement — attempt 2')
  })

  it('maps tool call segments and scopes by node', () => {
    const groups = buildRunTranscriptGroups([
      segment({ id: 's-other-node', node_id: 'review', turn_id: 'root:review:attempt-0' }),
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
    ], 'implement')
    expect(groups).toHaveLength(1)
    expect(groups[0].rows).toHaveLength(1)
    expect(groups[0].rows[0].kind).toBe('tool_call')
  })

  it('labels child-run groups with their flow', () => {
    const groups = buildRunTranscriptGroups([
      segment({
        id: 's-child',
        turn_id: 'run-child:child_step:attempt-0',
        node_id: 'child_step',
        source_scope: 'child',
        source_flow_name: 'child-flow.dot',
        source_run_id: 'run-child',
      }),
    ])
    expect(runTranscriptGroupLabel(groups[0])).toBe('child_step (child-flow.dot)')
  })
})
