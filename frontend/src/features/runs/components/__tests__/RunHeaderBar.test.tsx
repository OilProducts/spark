import { render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'

import { RunDetailsCard } from '../RunDetailsCard'
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

const renderHeader = (run: RunRecord, onFocusPendingQuestions: (() => void) | null = null) => render(
    <RunHeaderBar
        run={run}
        now={Date.parse('2026-03-22T00:10:00Z')}
        currentNodeId={run.current_node ?? null}
        onRequestCancel={vi.fn()}
        onRequestRetry={vi.fn()}
        onContinueFromRun={vi.fn()}
        onFocusPendingQuestions={onFocusPendingQuestions}
    />,
)

describe('RunHeaderBar', () => {
    it('shows identity, status, ambient facts, and actions on the strip', () => {
        renderHeader(makeRun())

        expect(screen.getByTestId('run-header-title')).toHaveTextContent('review.dot')
        expect(screen.getByTestId('run-header-status')).toHaveTextContent('Completed')
        expect(screen.getByTestId('run-header-fact-duration')).toHaveTextContent('5m')
        expect(screen.getByTestId('run-header-fact-tokens')).toHaveTextContent('1,234')
        expect(screen.getByTestId('run-summary-cancel-button')).toBeDisabled()
        expect(screen.getByTestId('run-summary-continue-button')).toBeVisible()
        expect(screen.queryByTestId('run-header-failure-reason')).not.toBeInTheDocument()
        expect(screen.queryByTestId('run-header-waiting-chip')).not.toBeInTheDocument()
    })

    it('leads with the failure reason on failed runs', () => {
        renderHeader(makeRun({
            status: 'failed',
            outcome: 'failure',
            last_error: 'tool command failed with code 1',
            current_node: 'transform',
        }))

        expect(screen.getByTestId('run-header-failure-reason')).toHaveTextContent('tool command failed with code 1')
        expect(screen.getByTestId('run-header-fact-node')).toHaveTextContent('transform')
        expect(screen.getByTestId('run-summary-retry-button')).toBeVisible()
    })

    it('offers a waiting chip that focuses the pending questions', () => {
        const onFocus = vi.fn()
        renderHeader(makeRun({ status: 'waiting', ended_at: null, current_node: 'review' }), onFocus)

        const chip = screen.getByTestId('run-header-waiting-chip')
        expect(chip).toHaveTextContent('Waiting for input at review')
        chip.click()
        expect(onFocus).toHaveBeenCalledTimes(1)
        // Waiting runs stay cancelable from the header.
        expect(screen.getByTestId('run-summary-cancel-button')).toBeEnabled()
    })
})

describe('RunDetailsCard', () => {
    it('displays execution placement metadata when a run has execution fields', () => {
        render(
            <RunDetailsCard
                run={makeRun({
                    execution_profile_id: 'local-dev',
                    execution_mode: 'local_container',
                    execution_container_image: 'spark-exec:latest',
                })}
                activeProjectPath="/tmp/project"
            />,
        )

        expect(screen.getByTestId('run-summary-section-execution')).toHaveTextContent('Execution')
        expect(screen.getByTestId('run-summary-execution-profile')).toHaveTextContent('local-dev')
        expect(screen.getByTestId('run-summary-execution-mode')).toHaveTextContent('local_container')
        expect(screen.getByTestId('run-summary-execution-container-image')).toHaveTextContent('spark-exec:latest')
    })

    it('omits the execution section for legacy runs without execution metadata', () => {
        render(<RunDetailsCard run={makeRun()} activeProjectPath="/tmp/project" />)

        expect(screen.queryByTestId('run-summary-section-execution')).not.toBeInTheDocument()
        expect(screen.getByTestId('run-summary-section-scope')).toHaveTextContent('review.dot')
        expect(screen.getByTestId('run-summary-token-usage')).toHaveTextContent('1,234')
    })
})
