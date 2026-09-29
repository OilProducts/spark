import { render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'

import { RunHeaderBar } from '../RunHeaderBar'
import type { RunRecord } from '../../model/shared'

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

const renderHeader = (run: RunRecord, selectedVisitLabel: string | null = null, recordedCommit: string | null = null) => render(
    <RunHeaderBar
        run={run}
        now={Date.parse('2026-03-22T00:10:00Z')}
        flowTitle="Review Changes"
        selectedVisitLabel={selectedVisitLabel}
        recordedCommit={recordedCommit}
        onRequestCancel={vi.fn()}
        onRequestRetry={vi.fn()}
        onContinueFromRun={vi.fn()}
        onRerunRun={vi.fn()}
    />,
)

describe('RunHeaderBar', () => {
    it('shows the run title under its flow and one line of facts', () => {
        renderHeader(makeRun({ title: 'Tighten the review loop' }))

        expect(screen.getByTestId('run-header-flow')).toHaveTextContent('Review Changes')
        expect(screen.getByTestId('run-header-title')).toHaveTextContent('Tighten the review loop')
        expect(screen.getByTestId('run-header-facts')).toHaveTextContent('Completed 5m ago · 5m · 1,234 tokens · abcdef0')
        expect(screen.queryByText(/Node:/)).not.toBeInTheDocument()
    })

    it('shows the commit the flow recorded when the run record has none', () => {
        renderHeader(makeRun({ git_commit: null, git_branch: null }), null, '6debdc97bfbdf337d60c301d4df6730b1d040262')
        expect(screen.getByTestId('run-header-facts')).toHaveTextContent('1,234 tokens · 6debdc9')
    })

    it('offers Retry only for failed runs and Cancel only for active ones', () => {
        const { unmount } = renderHeader(makeRun())
        expect(screen.queryByTestId('run-summary-retry-button')).not.toBeInTheDocument()
        expect(screen.queryByTestId('run-summary-cancel-button')).not.toBeInTheDocument()
        expect(screen.getByTestId('run-summary-rerun-button')).toBeVisible()
        expect(screen.getByTestId('run-summary-continue-button')).toHaveTextContent('Continue from here')
        unmount()

        const failed = renderHeader(makeRun({ status: 'failed', outcome: 'failure', last_error: 'boom' }), 'Transform')
        expect(screen.getByTestId('run-summary-retry-button')).toBeVisible()
        expect(screen.queryByTestId('run-summary-cancel-button')).not.toBeInTheDocument()
        expect(screen.getByTestId('run-summary-continue-button')).toHaveAttribute('title', 'Start a new run from Transform')
        failed.unmount()

        renderHeader(makeRun({ status: 'waiting', ended_at: null }))
        expect(screen.getByTestId('run-summary-cancel-button')).toBeEnabled()
        expect(screen.queryByTestId('run-summary-retry-button')).not.toBeInTheDocument()
        expect(screen.queryByTestId('run-summary-continue-button')).not.toBeInTheDocument()
        expect(screen.getByTestId('run-header-status')).toHaveTextContent('Needs input')
    })
})
