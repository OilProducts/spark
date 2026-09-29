import { RunList } from '@/features/runs/components/RunList'
import type { RunRecord } from '@/features/runs/model/shared'
import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'

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
    parent_run_id: overrides.parent_run_id ?? null,
    root_run_id: overrides.root_run_id ?? null,
    title: overrides.title ?? null,
    launch_context: overrides.launch_context ?? null,
})

describe('RunList', () => {
    it('shows the cold-load notice only when no runs have been loaded yet', () => {
        render(
            <RunList
                activeProjectPath="/tmp/project-one"
                error={null}
                onScopeModeChange={vi.fn()}
                onSelectRun={vi.fn()}
                runs={[]}
                scopeMode="active"
                selectedRunId={null}
                status="loading"
                summaryLabel="0 total runs · 0 running"
            />,
        )

        expect(screen.getByTestId('run-list-loading')).toBeVisible()
        expect(screen.queryByTestId('run-list-scroll-region')).not.toBeInTheDocument()
    })

    it('renders run rows without polling-era refresh affordances', () => {
        render(
            <RunList
                activeProjectPath="/tmp/project-one"
                error={null}
                onScopeModeChange={vi.fn()}
                onSelectRun={vi.fn()}
                runs={[makeRun({ flow_name: 'refreshing.dot' })]}
                scopeMode="active"
                selectedRunId={null}
                status="ready"
                summaryLabel="1 total runs · 1 running"
            />,
        )

        expect(screen.queryByTestId('run-list-loading')).not.toBeInTheDocument()
        expect(screen.getByTestId('run-list-scroll-region')).toBeVisible()
        expect(screen.getByText('Refreshing')).toBeVisible()
        expect(screen.queryByTestId('runs-refresh-button')).not.toBeInTheDocument()
    })

    it('titles runs, folds child runs under their parent, and filters on search', () => {
        const parent = makeRun({ run_id: 'parent', status: 'completed', title: 'Tighten the review loop', flow_name: 'software-development/implement-change.yaml' })
        const child = makeRun({
            run_id: 'child-run',
            flow_name: 'Implement Task',
            parent_run_id: 'parent',
            root_run_id: 'parent',
        })
        const untitled = makeRun({
            run_id: 'other',
            status: 'failed',
            flow_name: 'merge-change.yaml',
            launch_context: { 'context.request.artifact_path': 'changes/CR-1/request.md' },
        })
        render(
            <RunList
                activeProjectPath="/tmp/project-one"
                error={null}
                onScopeModeChange={vi.fn()}
                onSelectRun={vi.fn()}
                runs={[parent, child, untitled]}
                scopeMode="active"
                selectedRunId={null}
                status="ready"
                summaryLabel="3 total runs · 1 running"
            />,
        )
        const titles = () => screen.getAllByTestId('run-history-row-title').map((row) => row.textContent)
        expect(titles()).toEqual(['Tighten the review loop', 'Merge Change · changes/CR-1/request.md'])
        expect(screen.getAllByTestId('run-history-row-meta')[0]).toHaveTextContent(/^Implement Change · /)
        expect(screen.getAllByTestId('run-history-row-status').map((status) => status.textContent)).toEqual([' · Failed'])

        const toggle = screen.getByTestId('run-history-children-toggle')
        expect(toggle).toHaveTextContent('1 child run')
        fireEvent.click(toggle)
        expect(titles()).toEqual(['Tighten the review loop', 'Implement Task', 'Merge Change · changes/CR-1/request.md'])
        expect(screen.getAllByTestId('run-history-row-status').map((status) => status.textContent)).toEqual([' · Running', ' · Failed'])

        fireEvent.change(screen.getByTestId('run-list-search-input'), { target: { value: 'merge' } })
        expect(titles()).toEqual(['Merge Change · changes/CR-1/request.md'])
        fireEvent.change(screen.getByTestId('run-list-search-input'), { target: { value: 'nothing like it' } })
        expect(screen.getByTestId('run-list-search-empty')).toBeVisible()
    })
})
