import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'

import { RunHeaderBar } from '../RunHeaderBar'
import type { RunRecord } from '../../model/shared'
import { useStore } from '@/store'
import { triggerKey } from '@/lib/api/triggersApi'

const makeRun = (overrides: Partial<RunRecord> = {}): RunRecord => ({
    run_id: 'run-1',
    flow_name: 'review.dot',
    status: 'completed',
    outcome: 'success',
    outcome_reason_code: null,
    outcome_reason_message: null,
    working_directory: '/tmp/project',
    project_path: '/tmp/project',
    git_branch: 'main',
    git_commit: 'abcdef0',
    spec_id: null,
    plan_id: null,
    model: 'gpt-5.3-codex-spark',
    started_at: '2026-03-22T00:00:00Z',
    ended_at: '2026-03-22T00:05:00Z',
    last_error: undefined,
    token_usage: 1234,
    token_usage_breakdown: null,
    estimated_model_cost: null,
    current_node: null,
    continued_from_run_id: null,
    continued_from_node: null,
    continued_from_flow_mode: null,
    continued_from_flow_name: null,
    parent_run_id: null,
    parent_node_id: null,
    root_run_id: null,
    child_invocation_index: null,
    ...overrides,
})

const renderHeader = (run: RunRecord, selectedVisitLabel: string | null = null) => render(
    <RunHeaderBar
        run={run}
        now={Date.parse('2026-03-22T00:10:00Z')}
        flowTitle="Review Changes"
        selectedVisitLabel={selectedVisitLabel}
        onRequestCancel={vi.fn()}
        onRequestRetry={vi.fn()}
        onContinueFromRun={vi.fn()}
        onRerunRun={vi.fn()}
    />,
)

describe('RunHeaderBar', () => {
    it('names the trigger that started a run and opens it', () => {
        renderHeader(makeRun({ title: 'Tests sweep', launch_context: { 'context.spark_trigger': { trigger_id: 'trigger-1', trigger_name: 'Tests sweep', source_type: 'schedule' } } }))
        expect(screen.getByTestId('run-header-trigger')).toHaveTextContent('started by trigger Tests sweep')
        fireEvent.click(screen.getByRole('button', { name: 'Tests sweep' }))
        expect(useStore.getState().viewMode).toBe('triggers')
        expect(useStore.getState().triggersSession.selectedTriggerId).toBe(triggerKey({ project_path: '/tmp/project', id: 'trigger-1' }))
    })

    it('names a completed run that reached a failure exit by its result, as a failure', () => {
        renderHeader(makeRun({ outcome: 'failure', outcome_reason_code: 'blocked', outcome_reason_message: 'review chose blocked' }))
        expect(screen.getByTestId('run-header-status')).toHaveTextContent('Blocked')
        expect(screen.getByTestId('run-header-status')).toHaveAttribute('data-tone', 'destructive')
    })

    it('shows the run title under its flow, a status pill, and one line of facts', () => {
        renderHeader(makeRun({ title: 'Tighten the review loop' }))

        expect(screen.getByTestId('run-header-flow')).toHaveTextContent('Review Changes')
        expect(screen.getByTestId('run-header-title')).toHaveTextContent('Tighten the review loop')
        expect(screen.getByTestId('run-header-status')).toHaveTextContent('Completed')
        expect(screen.getByTestId('run-header-status')).toHaveAttribute('data-tone', 'success')
        expect(screen.getByTestId('run-header-facts')).toHaveTextContent(/^\S.* · 5m 00s · 1\.2k tokens · abcdef0$/)
        expect(screen.queryByText(/Node:/)).not.toBeInTheDocument()
    })

    it('leads the breadcrumb with the project when given one', () => {
        render(
            <RunHeaderBar
                run={makeRun({ title: 'Tighten the review loop' })}
                now={Date.parse('2026-03-22T00:10:00Z')}
                flowTitle="Review Changes"
                project="spark"
                selectedVisitLabel={null}
                onRequestCancel={vi.fn()}
                onRequestRetry={vi.fn()}
                onContinueFromRun={vi.fn()}
                onRerunRun={vi.fn()}
            />,
        )
        expect(screen.getByTestId('run-header-flow').parentElement).toHaveTextContent('spark · Review Changes')
    })

    it('drops the flow line when the title is the flow title', () => {
        renderHeader(makeRun({ title: 'Review Changes' }))

        expect(screen.queryByTestId('run-header-flow')).not.toBeInTheDocument()
        expect(screen.getByTestId('run-header-title')).toHaveTextContent('Review Changes')
    })

    it('offers Retry only for failed runs and Cancel only for active ones', () => {
        const { unmount } = renderHeader(makeRun())
        expect(screen.queryByTestId('run-summary-retry-button')).not.toBeInTheDocument()
        expect(screen.queryByTestId('run-summary-cancel-button')).not.toBeInTheDocument()
        expect(screen.getByTestId('run-summary-rerun-button')).toBeVisible()
        // With no visit selected, Continue asks for the node to start from.
        expect(screen.getByTestId('run-summary-continue-button')).toHaveTextContent('Continue from…')
        // The actions explain themselves without hovering, naming only the ones shown.
        expect(screen.getByTestId('run-header-actions-help')).toHaveTextContent(
            'Continue from… starts a new run from a step you pick, keeping results so far. Re-run starts over with the same inputs.',
        )
        unmount()

        const failed = renderHeader(makeRun({ status: 'failed', outcome: 'failure', last_error: 'boom' }), 'Transform')
        expect(screen.getByTestId('run-summary-retry-button')).toBeVisible()
        expect(screen.queryByTestId('run-summary-cancel-button')).not.toBeInTheDocument()
        expect(screen.getByTestId('run-summary-continue-button')).toHaveTextContent('Continue from here')
        // Each recovery action says how it differs from the others.
        expect(screen.getByTestId('run-summary-retry-button')).toHaveAttribute('title', expect.stringContaining('from the step where it failed, with the same flow'))
        expect(screen.getByTestId('run-summary-continue-button')).toHaveAttribute('title', 'Start a new run from Transform, reusing this run’s results so far')
        expect(screen.getByTestId('run-summary-rerun-button')).toHaveAttribute('title', expect.stringContaining('from the beginning, with the same inputs'))
        // Focus reads each button's own sentence from the visible line.
        expect(screen.getByTestId('run-summary-retry-button')).toHaveAccessibleDescription('Retry picks up from the failed step.')
        expect(screen.getByTestId('run-summary-continue-button')).toHaveAccessibleDescription('Continue from here starts a new run from Transform, keeping results so far.')
        expect(screen.getByTestId('run-summary-rerun-button')).toHaveAccessibleDescription('Re-run starts over with the same inputs.')
        failed.unmount()

        renderHeader(makeRun({ status: 'waiting', ended_at: null }))
        expect(screen.getByTestId('run-summary-cancel-button')).toBeEnabled()
        expect(screen.queryByTestId('run-summary-retry-button')).not.toBeInTheDocument()
        expect(screen.queryByTestId('run-summary-continue-button')).not.toBeInTheDocument()
        expect(screen.queryByTestId('run-header-actions-help')).not.toBeInTheDocument()
        expect(screen.getByTestId('run-header-status')).toHaveTextContent('Needs input')
        expect(screen.getByTestId('run-header-status')).toHaveAttribute('data-tone', 'warning')
        // A waiting run states its start like a finished one, then how long it has been going.
        expect(screen.getByTestId('run-header-facts')).toHaveTextContent(/^\S.* · 10m 00s · 1\.2k tokens · abcdef0$/)
    })

    it('uses the filled button only for Retry; the other commands are outlined', () => {
        const failed = makeRun({ status: 'failed', outcome: 'failure', last_error: 'boom' })
        const { unmount } = renderHeader(failed, 'Transform')
        expect(screen.getByTestId('run-summary-retry-button')).toHaveAttribute('data-variant', 'default')
        expect(screen.getByTestId('run-summary-continue-button')).toHaveAttribute('data-variant', 'outline')
        expect(screen.getByTestId('run-summary-rerun-button')).toHaveAttribute('data-variant', 'outline')
        unmount()

        // While a continuation draft is open, its commit button is the one filled button.
        render(
            <RunHeaderBar
                run={failed}
                now={Date.parse('2026-03-22T00:10:00Z')}
                flowTitle="Review Changes"
                selectedVisitLabel="Transform"
                continuationOpen
                onRequestCancel={vi.fn()}
                onRequestRetry={vi.fn()}
                onContinueFromRun={vi.fn()}
                onRerunRun={vi.fn()}
            />,
        )
        expect(screen.getByTestId('run-summary-retry-button')).toHaveAttribute('data-variant', 'outline')
    })
})
