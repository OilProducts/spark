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
})
