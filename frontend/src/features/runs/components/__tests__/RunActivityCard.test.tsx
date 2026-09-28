import { render, screen, within } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'

import type { RunTranscriptSegment } from '@/lib/api/attractorApi'
import type { GroupedTimelineEntry, TimelineEventEntry } from '../../model/shared'
import { RunActivityCard } from '../RunActivityCard'

const segment = (stageIndex: number, content: string, at: string): RunTranscriptSegment => ({
    id: 'final-response',
    turn_id: 'response',
    order: 1,
    kind: 'assistant_message',
    role: 'assistant',
    status: 'complete',
    timestamp: at,
    updated_at: at,
    content,
    node_id: 'evaluate',
    stage_index: stageIndex,
    attempt: 0,
    // Execution-local sequence numbers: far larger than the journal's, so
    // ordering on them would push every transcript group to the bottom.
    latest_sequence: 900 + stageIndex,
    source_scope: 'root',
    source_flow_name: null,
    source_parent_node_id: null,
    source_run_id: 'run-1',
})

const event = (sequence: number, summary: string, at: string): TimelineEventEntry => ({
    id: `event-${sequence}`,
    sequence,
    type: 'StageCompleted',
    category: 'stage',
    severity: 'info',
    nodeId: 'evaluate',
    stageIndex: null,
    summary,
    receivedAt: at,
    sourceScope: 'root',
    sourceParentNodeId: null,
    sourceFlowName: null,
    payload: {},
})

describe('RunActivityCard', () => {
    it('interleaves execution transcripts and run events in time order', () => {
        const events: GroupedTimelineEntry[] = [{
            id: 'events',
            correlation: null,
            events: [
                event(2, 'First visit done', '2026-07-08T10:02:00Z'),
                event(4, 'Second visit done', '2026-07-08T10:06:00Z'),
            ],
        }]
        render(
            <RunActivityCard
                isNarrowViewport={false}
                isLive={false}
                activityMode="all"
                onActivityModeChange={vi.fn()}
                selectedNodeId={null}
                onClearNodeSelection={vi.fn()}
                transcriptSegments={[
                    segment(5, 'Second verdict.', '2026-07-08T10:05:00Z'),
                    segment(1, 'First verdict.', '2026-07-08T10:01:00Z'),
                ]}
                transcriptError={null}
                groupedTimelineEntries={events}
                timelineError={null}
                timelineEventCount={2}
                filteredTimelineEventCount={2}
                timelineCategoryFilter="all"
                timelineSeverityFilter="all"
                onTimelineCategoryFilterChange={vi.fn()}
                onTimelineSeverityFilterChange={vi.fn()}
                hasOlderTimelineEvents={false}
                isTimelineLoadingOlder={false}
                onLoadOlderTimelineEvents={vi.fn()}
            />,
        )
        const rows = Array.from(screen.getByTestId('run-activity-list').children)
        expect(rows.map((row) => row.textContent)).toEqual([
            expect.stringContaining('First verdict.'),
            expect.stringContaining('First visit done'),
            expect.stringContaining('Second verdict.'),
            expect.stringContaining('Second visit done'),
        ])
        expect(within(rows[0] as HTMLElement).getByText('evaluate — visit 1')).toBeVisible()
        expect(within(rows[2] as HTMLElement).getByText('evaluate — visit 2')).toBeVisible()
    })
})
