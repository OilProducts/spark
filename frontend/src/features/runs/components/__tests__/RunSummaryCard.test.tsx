import { render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'

import { RunSummaryCard } from '../RunSummaryCard'
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

const renderSummary = (run: RunRecord) => render(
    <RunSummaryCard
        run={run}
        activeProjectPath="/tmp/project"
        now={Date.parse('2026-03-22T00:10:00Z')}
        collapsed={false}
        monitoringFacts={[]}
        monitoringHeadline="Completed"
        onRequestCancel={vi.fn()}
        onRequestRetry={vi.fn()}
        onContinueFromRun={vi.fn()}
        onCollapsedChange={vi.fn()}
    />,
)

describe('RunSummaryCard', () => {
    it('displays execution placement metadata when a run has execution fields', () => {
        renderSummary(makeRun({
            execution_profile_id: 'local-dev',
            execution_mode: 'local_container',
            execution_container_image: 'spark-exec:latest',
        }))

        expect(screen.getByTestId('run-summary-section-execution')).toHaveTextContent('Execution')
        expect(screen.getByTestId('run-summary-execution-profile')).toHaveTextContent('local-dev')
        expect(screen.getByTestId('run-summary-execution-mode')).toHaveTextContent('local_container')
        expect(screen.getByTestId('run-summary-execution-container-image')).toHaveTextContent('spark-exec:latest')
    })

    it('omits the execution section for legacy runs without execution metadata', () => {
        renderSummary(makeRun())

        expect(screen.queryByTestId('run-summary-section-execution')).not.toBeInTheDocument()
    })
})
