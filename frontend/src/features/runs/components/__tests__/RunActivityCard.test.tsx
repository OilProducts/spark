import { RunActivityCard } from '@/features/runs/components/RunActivityCard'
import type { GroupedTimelineEntry, RunRecord } from '@/features/runs/model/shared'
import { useStore } from '@/store'
import { act, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { useState } from 'react'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'

const makeRun = (overrides: Partial<RunRecord> = {}): RunRecord => ({
    run_id: overrides.run_id ?? 'run-1',
    flow_name: overrides.flow_name ?? 'selected.dot',
    status: overrides.status ?? 'running',
    outcome: overrides.outcome ?? null,
    outcome_reason_code: overrides.outcome_reason_code ?? null,
    outcome_reason_message: overrides.outcome_reason_message ?? null,
    working_directory: overrides.working_directory ?? '/tmp/workdir',
    project_path: overrides.project_path ?? '/tmp/project-one',
    git_branch: overrides.git_branch ?? 'main',
    git_commit: overrides.git_commit ?? 'abcdef0',
    spec_id: overrides.spec_id ?? null,
    plan_id: overrides.plan_id ?? null,
    model: overrides.model ?? 'gpt-5.4',
    started_at: overrides.started_at ?? '2026-03-22T00:00:00Z',
    ended_at: overrides.ended_at ?? null,
    last_error: overrides.last_error ?? '',
    token_usage: overrides.token_usage ?? 1234,
    current_node: overrides.current_node ?? null,
    continued_from_run_id: overrides.continued_from_run_id ?? null,
    continued_from_node: overrides.continued_from_node ?? null,
    continued_from_flow_mode: overrides.continued_from_flow_mode ?? null,
    continued_from_flow_name: overrides.continued_from_flow_name ?? null,
})

const makeGroupedEntry = (overrides: Partial<GroupedTimelineEntry> = {}): GroupedTimelineEntry => ({
    id: overrides.id ?? 'entry-1',
    correlation: overrides.correlation ?? null,
    events: overrides.events ?? [
        {
            id: 'event-1',
            sequence: 1,
            type: 'StageStarted',
            category: 'stage',
            severity: 'info',
            nodeId: 'validate',
            stageIndex: 2,
            summary: 'Stage validate started',
            receivedAt: '2026-03-22T00:02:00Z',
            payload: {},
        },
    ],
})

const resetActivityState = () => {
    act(() => {
        useStore.setState((state) => ({
            ...state,
            logs: [],
            selectedRunCompletedNodes: [],
        }))
    })
}

function ControlledRunActivityCard(props: {
    checkpointCompletedNodes: string
    checkpointCurrentNode: string
    checkpointRetryCounters: string
    groupedTimelineEntries: GroupedTimelineEntry[]
    pendingGateCount: number
    run: RunRecord
}) {
    const [collapsed, setCollapsed] = useState(false)
    const [rawLogsCollapsed, setRawLogsCollapsed] = useState(true)

    return (
        <RunActivityCard
            {...props}
            collapsed={collapsed}
            rawLogsCollapsed={rawLogsCollapsed}
            onCollapsedChange={setCollapsed}
            onRawLogsCollapsedChange={setRawLogsCollapsed}
        />
    )
}

describe('RunActivityCard', () => {
    beforeEach(() => {
        resetActivityState()
    })

    afterEach(() => {
        resetActivityState()
    })

    it('renders the expected operator headlines across key run states', () => {
        const { rerender } = render(
            <RunActivityCard
                checkpointCompletedNodes="start"
                checkpointCurrentNode="validate"
                checkpointRetryCounters="—"
                collapsed={false}
                groupedTimelineEntries={[]}
                pendingGateCount={1}
                rawLogsCollapsed={true}
                run={makeRun({ status: 'running', current_node: 'review_gate' })}
                onCollapsedChange={() => {}}
                onRawLogsCollapsedChange={() => {}}
            />,
        )

        expect(screen.getByTestId('run-activity-headline')).toHaveTextContent('Waiting for input at review_gate')

        rerender(
            <RunActivityCard
                checkpointCompletedNodes="start"
                checkpointCurrentNode="validate"
                checkpointRetryCounters="—"
                collapsed={false}
                groupedTimelineEntries={[]}
                pendingGateCount={0}
                rawLogsCollapsed={true}
                run={makeRun({ status: 'running', current_node: 'validate' })}
                onCollapsedChange={() => {}}
                onRawLogsCollapsedChange={() => {}}
            />,
        )
        expect(screen.getByTestId('run-activity-headline')).toHaveTextContent('Running validate')

        rerender(
            <RunActivityCard
                checkpointCompletedNodes="start"
                checkpointCurrentNode="validate"
                checkpointRetryCounters="—"
                collapsed={false}
                groupedTimelineEntries={[]}
                pendingGateCount={0}
                rawLogsCollapsed={true}
                run={makeRun({ status: 'cancel_requested', current_node: 'validate' })}
                onCollapsedChange={() => {}}
                onRawLogsCollapsedChange={() => {}}
            />,
        )
        expect(screen.getByTestId('run-activity-headline')).toHaveTextContent('Cancel requested while validate winds down')

        rerender(
            <RunActivityCard
                checkpointCompletedNodes="start, done"
                checkpointCurrentNode="done"
                checkpointRetryCounters="—"
                collapsed={false}
                groupedTimelineEntries={[]}
                pendingGateCount={0}
                rawLogsCollapsed={true}
                run={makeRun({ status: 'completed', outcome: 'success', current_node: 'done', ended_at: '2026-03-22T00:05:00Z' })}
                onCollapsedChange={() => {}}
                onRawLogsCollapsedChange={() => {}}
            />,
        )
        expect(screen.getByTestId('run-activity-headline')).toHaveTextContent('Completed successfully')

        rerender(
            <RunActivityCard
                checkpointCompletedNodes="start, done"
                checkpointCurrentNode="done"
                checkpointRetryCounters="—"
                collapsed={false}
                groupedTimelineEntries={[]}
                pendingGateCount={0}
                rawLogsCollapsed={true}
                run={makeRun({
                    status: 'completed',
                    outcome: 'failure',
                    current_node: 'done',
                    outcome_reason_message: 'Missing release credentials',
                    ended_at: '2026-03-22T00:05:00Z',
                })}
                onCollapsedChange={() => {}}
                onRawLogsCollapsedChange={() => {}}
            />,
        )
        expect(screen.getByTestId('run-activity-headline')).toHaveTextContent('Completed with failure outcome: Missing release credentials')

        rerender(
            <RunActivityCard
                checkpointCompletedNodes="start"
                checkpointCurrentNode="validate"
                checkpointRetryCounters="—"
                collapsed={false}
                groupedTimelineEntries={[]}
                pendingGateCount={0}
                rawLogsCollapsed={true}
                run={makeRun({ status: 'validation_error', current_node: 'validate', last_error: 'Pytest failed' })}
                onCollapsedChange={() => {}}
                onRawLogsCollapsedChange={() => {}}
            />,
        )
        expect(screen.getByTestId('run-activity-headline')).toHaveTextContent('Failed in validate: Pytest failed')
    })

    it('shows the facts strip, recent activity feed, and collapsible raw logs', async () => {
        act(() => {
            useStore.setState((state) => ({
                ...state,
                selectedRunCompletedNodes: ['start', 'plan'],
                logs: [
                    { time: '10:00:00', msg: 'Stage started', type: 'info' },
                    { time: '10:00:05', msg: 'Stage completed successfully', type: 'success' },
                ],
            }))
        })

        const user = userEvent.setup()
        render(
            <ControlledRunActivityCard
                checkpointCompletedNodes="start, plan"
                checkpointCurrentNode="validate"
                checkpointRetryCounters="validate: 1"
                groupedTimelineEntries={[
                    makeGroupedEntry({
                        id: 'entry-1',
                        events: [
                            {
                                id: 'event-1',
                                sequence: 1,
                                type: 'StageRetrying',
                                category: 'stage',
                                severity: 'warning',
                                nodeId: 'validate',
                                stageIndex: 3,
                                summary: 'Stage validate retrying (attempt 2)',
                                receivedAt: '2026-03-22T00:03:00Z',
                                payload: { attempt: 2 },
                            },
                        ],
                    }),
                    makeGroupedEntry({
                        id: 'entry-2',
                        events: [
                            {
                                id: 'event-2',
                                sequence: 2,
                                type: 'StageCompleted',
                                category: 'stage',
                                severity: 'info',
                                nodeId: 'plan',
                                stageIndex: 2,
                                summary: 'Stage plan completed',
                                receivedAt: '2026-03-22T00:02:00Z',
                                payload: {},
                            },
                        ],
                    }),
                ]}
                pendingGateCount={2}
                run={makeRun({ status: 'running', current_node: 'validate' })}
            />,
        )

        expect(screen.getByTestId('run-activity-fact-current-node')).toHaveTextContent('validate')
        expect(screen.getByTestId('run-activity-fact-completed-count')).toHaveTextContent('2')
        expect(screen.getByTestId('run-activity-fact-pending-gates')).toHaveTextContent('2')
        expect(screen.getByTestId('run-activity-fact-retry-state')).toHaveTextContent('Retrying validate (attempt 2)')
        expect(screen.getAllByTestId('run-activity-entry')).toHaveLength(2)
        expect(screen.getAllByTestId('run-activity-entry-summary')[0]).toHaveTextContent('Stage validate retrying (attempt 2)')

        expect(screen.queryByTestId('run-activity-logs-panel')).not.toBeInTheDocument()

        await user.click(screen.getByTestId('run-activity-logs-toggle-button'))

        expect(screen.getByTestId('run-activity-logs-panel')).toBeVisible()
        expect(screen.getAllByTestId('run-activity-log-row')).toHaveLength(2)

        await user.click(screen.getByTestId('run-activity-clear-logs-button'))

        expect(screen.getByTestId('run-activity-logs-panel')).toHaveTextContent('No runtime logs have arrived for this run yet.')
    })
})
