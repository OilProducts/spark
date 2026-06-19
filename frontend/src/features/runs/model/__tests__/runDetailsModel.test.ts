import {
  asPendingQuestionSnapshot,
  buildArtifactDerivedState,
  buildCheckpointSummary,
  buildContextRows,
  filterContextRows,
} from '@/features/runs/model/runDetailsModel'

describe('runDetailsModel', () => {
  it('builds pending question snapshots with semantic fallback options', () => {
    const snapshot = asPendingQuestionSnapshot({
      question_id: 'gate-1',
      node_id: 'human-review',
      prompt: 'Ship this change?',
      question_type: 'YES_NO',
      options: [],
    })

    expect(snapshot).toMatchObject({
      questionId: 'gate-1',
      nodeId: 'human-review',
      questionType: 'YES_NO',
      options: [
        { label: 'Yes', value: 'YES', key: 'Y', description: null },
        { label: 'No', value: 'NO', key: 'N', description: null },
      ],
    })
  })

  it('formats and filters structured context rows', () => {
    const rows = buildContextRows({
      pipeline_id: 'run-1',
      context: {
        summary: 'done',
        details: { result: 'ok', retries: 1 },
      },
    })

    expect(rows.map((row) => row.key)).toEqual(['details', 'summary'])
    expect(rows[0]?.renderKind).toBe('structured')
    expect(filterContextRows(rows, 'retries')).toEqual([rows[0]])
  })

  it('derives missing core artifacts and selected artifact state', () => {
    const derived = buildArtifactDerivedState({
      pipeline_id: 'run-1',
      artifacts: [
        { path: 'manifest.json', size_bytes: 120, media_type: 'application/json', viewable: true },
        { path: 'logs/output.txt', size_bytes: 48, media_type: 'text/plain', viewable: true },
      ],
    }, 'logs/output.txt')

    expect(derived.missingCoreArtifacts).toEqual(['checkpoint.json'])
    expect(derived.selectedArtifactEntry?.path).toBe('logs/output.txt')
    expect(derived.showPartialRunArtifactNote).toBe(true)
  })

  it('summarizes checkpoint state for the detail cards', () => {
    const summary = buildCheckpointSummary({
      pipeline_id: 'run-1',
      checkpoint: {
        active_node: 'apply',
        last_completed_node: 'review',
        completed_nodes: ['plan', 'review'],
        retry_counts: { apply: 2 },
      },
    })

    expect(summary).toEqual({
      checkpointCompletedNodes: 'plan, review',
      checkpointCurrentNode: 'apply',
      checkpointRetryCounters: 'apply: 2',
    })
  })

  it('does not use a terminal checkpoint last completed node as the resume node', () => {
    const summary = buildCheckpointSummary({
      pipeline_id: 'run-1',
      checkpoint: {
        active_node: null,
        last_completed_node: 'done',
        completed_nodes: ['plan', 'review', 'done'],
        retry_counts: {},
      },
    })

    expect(summary).toMatchObject({
      checkpointCompletedNodes: 'plan, review, done',
      checkpointCurrentNode: '—',
    })
  })
})
