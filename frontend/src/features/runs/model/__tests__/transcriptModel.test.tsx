import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { useRunTranscriptStore } from '../../state/runTranscriptStore'

import type { RunTranscriptSegment } from '@/lib/api/attractorApi'
import { Transcript } from '@/components/app/transcript/Transcript'
import type { ConversationTimelineEntry } from '@/features/projects/model/types'
import { formatTimestamp } from '../shared'
import { visitTranscriptEntries, type RunVisit } from '../visitModel'

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

// The visit only scopes segments to its execution (node, visit, attempt, run).
const visit = { nodeId: 'implement', stageIndex: 0, attempt: 0, runId: 'run-1', parentKey: null } as RunVisit
const entriesOf = (segments: RunTranscriptSegment[]) => visitTranscriptEntries(visit, segments.map((entry, order) => ({ ...entry, order, source_run_id: 'run-1' })))
const RunTranscript = ({ entries }: { entries: ConversationTimelineEntry[] }) => (
  <Transcript testIdPrefix="run" turns={false} entries={entries} showThinking={false} formatTimestamp={formatTimestamp} />
)

describe('visitTranscriptEntries', () => {
  it('maps messages, reasoning, tool calls, compaction and warnings onto timeline entries and skips other kinds', () => {
    const entries = entriesOf([
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
      segment({
        id: 's-question',
        kind: 'request_user_input',
        role: 'system',
        request_user_input: { request_id: 'q', status: 'pending', questions: [], answers: {}, submitted_at: null },
      }),
      segment({ id: 's-launch', kind: 'flow_launch', role: 'system', content: '', artifact_id: 'launch-1' }),
      segment({ id: 's-recovery', kind: 'run_recovery', role: 'system', content: '', artifact_id: 'run-recovery-1' }),
    ])
    expect(entries.map((entry) => entry.id)).toEqual(['s-answer', 's-thinking', 's-tool', 's-compaction', 's-warning'])
    expect(entries).toEqual([
      expect.objectContaining({ kind: 'message', role: 'assistant', content: 'Done.' }),
      expect.objectContaining({ kind: 'message', presentation: 'thinking' }),
      expect.objectContaining({ kind: 'tool_call', toolCall: expect.objectContaining({ id: 'call-1', command: 'ls -la' }) }),
      expect.objectContaining({ kind: 'context_compaction', content: 'Context compacted.' }),
      expect.objectContaining({ kind: 'warning', content: 'API error 529; retrying' }),
    ])
    // No turn id, so a run node offers no retry.
    expect(entries[0]).toMatchObject({ turnId: undefined })
  })

  it('renders compaction and warning rows as event lines', () => {
    render(<RunTranscript entries={entriesOf([
      segment({ id: 's-compaction', kind: 'context_compaction', role: 'system', content: 'Context compacted (21.6k \u2192 1.9k tokens).' }),
      segment({ id: 's-warning', kind: 'agent_event', category: 'warning', role: 'system', content: 'Rate limited until 15:00 UTC' }),
    ])} />)
    expect(screen.getByTestId('run-context-compaction-row-s-compaction')).toHaveTextContent('Context compacted (21.6k \u2192 1.9k tokens).')
    expect(screen.getByTestId('run-warning-row-s-warning')).toHaveTextContent('Rate limited until 15:00 UTC')
  })

  it('shows a steer as the user\'s message, and one the agent never read as not sent', () => {
    render(<RunTranscript entries={entriesOf([
      segment({ id: 'segment-user-message-1', kind: 'user_message', role: 'user', content: 'Use the docs.' }),
      segment({
        id: 'segment-user-message-2',
        kind: 'user_message',
        role: 'user',
        status: 'failed',
        content: 'Too late.',
        error: 'Not sent: the turn ended before the agent read it.',
      }),
    ])} />)
    const sent = screen.getByText('Use the docs.').closest('li')!
    expect(sent).toHaveTextContent('Sent into the running turn')
    const unread = screen.getByText('Too late.').closest('li')!
    expect(unread).toHaveTextContent('Not sent: the turn ended before the agent read it.')
  })
})

describe('RunTranscript subagent nesting', () => {
  const toolSegment = (id: string, toolCall: Partial<NonNullable<RunTranscriptSegment['tool_call']>>) => segment({
    id: `s-${id}`,
    kind: 'tool_call',
    role: 'system',
    tool_call: { id, kind: 'command_execution', status: 'completed', title: id, command: `run ${id}`, file_paths: [], ...toolCall },
  })

  it('renders a subagent\'s calls nested under its row, collapsed behind a count', () => {
    const rows = [
      toolSegment('agent', { kind: 'subagent', title: 'Find the config loader', command: null, subagent_type: 'Explore', summary: 'Found it' }),
      toolSegment('grep', { kind: 'dynamic_tool', title: 'Grep', command: null, file_paths: ['/src'], parent_call_id: 'agent' }),
      toolSegment('read', { kind: 'dynamic_tool', title: 'Read', command: null, file_paths: ['/src/config.rs'], parent_call_id: 'agent' }),
      toolSegment('top', {}),
    ]
    render(<RunTranscript entries={entriesOf(rows)} />)

    // Top-level calls still fold into one group; nested calls are not counted there.
    const group = screen.getByTestId('run-tool-group-agent')
    expect(group).toHaveTextContent('Ran 1 command, used 1 tool')
    fireEvent.click(group)
    const agent = screen.getByTestId('run-tool-call-toggle-agent')
    expect(agent).toHaveTextContent('Find the config loader')
    expect(agent).toHaveTextContent('Explore')
    expect(agent).toHaveTextContent('Found it')
    expect(screen.getByTestId('run-tool-call-toggle-top')).toBeInTheDocument()

    const nested = screen.getByTestId('run-tool-call-children-agent')
    expect(nested).toHaveTextContent('2 tool calls')
    expect(nested).toHaveAttribute('aria-expanded', 'false')
    expect(screen.queryByTestId('run-tool-call-toggle-grep')).toBeNull()
    fireEvent.click(nested)
    expect(nested.closest('li')).toContainElement(screen.getByTestId('run-tool-call-toggle-grep'))
    expect(screen.getByTestId('run-tool-call-toggle-read')).toHaveTextContent('/src/config.rs')
  })

  it('shows the subagent\'s reported tool count when fewer calls arrived', () => {
    const rows = [
      toolSegment('agent', { kind: 'subagent', title: 'Explore', command: null, tool_uses: 3 }),
      toolSegment('grep', { kind: 'dynamic_tool', title: 'Grep', command: null, parent_call_id: 'agent' }),
      toolSegment('solo', { kind: 'subagent', title: 'Background', command: null, tool_uses: 2 }),
    ]
    render(<RunTranscript entries={entriesOf(rows)} />)
    fireEvent.click(screen.getByTestId('run-tool-group-agent'))
    expect(screen.getByTestId('run-tool-call-children-agent')).toHaveTextContent('3 tool calls')
    expect(screen.getByTestId('run-tool-call-children-solo')).toHaveTextContent('2 tool calls')
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
