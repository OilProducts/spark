import { describe, expect, it } from 'vitest'

import { useRunTranscriptStore } from '../../state/runTranscriptStore'

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
  stage_index: 0,
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

  it('keeps executions that differ only in stage_index apart and labels each visit', () => {
    const shared = { turn_id: 'response', id: 'final-response', node_id: 'evaluate', source_run_id: 'run-1' }
    const groups = buildRunTranscriptGroups([
      segment({ ...shared, stage_index: 7, content: 'Second.', updated_at: '2026-07-08T10:05:00Z' }),
      segment({ ...shared, stage_index: 3, content: 'First.', updated_at: '2026-07-08T10:01:00Z' }),
    ])
    expect(groups.map((group) => group.rows[0].entry)).toMatchObject([{ content: 'First.' }, { content: 'Second.' }])
    expect(groups.map(runTranscriptGroupLabel)).toEqual(['evaluate — visit 1', 'evaluate — visit 2'])
    expect(new Set(groups.map((group) => group.key)).size).toBe(2)
  })

  it('keeps a root and a child run with matching node ids apart', () => {
    const shared = { turn_id: 'response', id: 'final-response', node_id: 'implement' }
    const groups = buildRunTranscriptGroups([
      segment({ ...shared, source_run_id: 'run-root', content: 'Root.' }),
      segment({ ...shared, source_run_id: 'run-child', source_scope: 'child', content: 'Child.' }),
    ])
    expect(groups).toHaveLength(2)
    expect(groups.map((group) => group.visit)).toEqual([null, null])
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
    expect(buildRunTranscriptGroups(useRunTranscriptStore.getState().byRunId['run-2'].segments)).toHaveLength(2)
  })
})
