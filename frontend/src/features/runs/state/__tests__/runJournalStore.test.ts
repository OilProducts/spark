import type { TimelineEventEntry } from '@/features/runs/model/shared'
import {
  flattenRunJournalSegments,
  useRunJournalStore,
} from '@/features/runs/state/runJournalStore'
import { beforeEach, describe, expect, it } from 'vitest'

const makeTimelineEvent = (sequence: number): TimelineEventEntry => ({
  id: `event-${sequence}`,
  sequence,
  type: 'StageStarted',
  category: 'stage',
  severity: 'info',
  nodeId: `stage_${sequence}`,
  stageIndex: sequence,
  summary: `Stage stage_${sequence} started`,
  receivedAt: new Date(Date.UTC(2026, 2, 22, 0, 0, sequence)).toISOString(),
  sourceScope: 'root',
  sourceParentNodeId: null,
  sourceFlowName: null,
  questionId: null,
  payload: {},
})

describe('runJournalStore', () => {
  beforeEach(() => {
    useRunJournalStore.setState({ byRunId: {} })
  })

  it('pages live updates into bounded live segments instead of growing one hot segment forever', () => {
    const runId = 'run-live-segment-paging'

    for (let sequence = 1; sequence <= 235; sequence += 1) {
      useRunJournalStore.getState().appendLiveEntry(runId, makeTimelineEvent(sequence))
    }

    const runState = useRunJournalStore.getState().byRunId[runId]
    expect(runState).toBeTruthy()
    expect(runState?.loadedEntryCount).toBe(235)
    expect(runState?.latestEntry?.sequence).toBe(235)

    const liveSegments = runState?.segments.filter((segment) => segment.role === 'live') ?? []
    expect(liveSegments).toHaveLength(3)
    expect(liveSegments.map((segment) => segment.entries.length)).toEqual([35, 100, 100])
    expect(liveSegments.every((segment) => segment.entries.length <= 100)).toBe(true)

    expect(
      flattenRunJournalSegments(runState?.segments ?? []).map(({ sequence }) => sequence),
    ).toEqual(Array.from({ length: 235 }, (_, index) => 235 - index))
  })

  it('keeps a live gate answerable when its human_gate event follows InterviewStarted', () => {
    const runId = 'run-live-gate'
    const gateEvent = (sequence: number, type: string, payload: Record<string, unknown>): TimelineEventEntry => ({
      ...makeTimelineEvent(sequence),
      type,
      category: 'interview',
      nodeId: 'approve',
      stageIndex: null,
      payload,
    })
    useRunJournalStore.getState().appendLiveEntry(runId, gateEvent(1, 'InterviewStarted', { question: 'Ship it?' }))
    useRunJournalStore.getState().appendLiveEntry(runId, gateEvent(2, 'human_gate', {
      question_id: 'q-1',
      prompt: 'Ship it?',
      options: [{ label: 'Yes', value: 'yes' }, { label: 'No', value: 'no' }],
    }))

    const gates = useRunJournalStore.getState().byRunId[runId]?.pendingInterviewGates ?? []
    expect(gates).toHaveLength(1)
    expect(gates[0]?.questionId).toBe('q-1')
    expect(gates[0]?.options.map((option) => option.value)).toEqual(['yes', 'no'])
  })
})
