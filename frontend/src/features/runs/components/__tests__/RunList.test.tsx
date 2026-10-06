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
    execution_lock: overrides.execution_lock ?? null,
})

describe('RunList', () => {
    it('shows the cold-load notice only when no runs have been loaded yet', () => {
        render(
            <RunList
                error={null}
                onSelectRun={vi.fn()}
                runs={[]}
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
                error={null}
                onSelectRun={vi.fn()}
                runs={[makeRun({ flow_name: 'refreshing.dot' })]}
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
                error={null}
                onSelectRun={vi.fn()}
                runs={[parent, child, untitled]}
                selectedRunId={null}
                status="ready"
                summaryLabel="3 total runs · 1 running"
            />,
        )
        const titles = () => screen.getAllByTestId('run-history-row-title').map((row) => row.textContent)
        expect(titles()).toEqual(['Tighten the review loop', 'changes/CR-1/request.md'])
        // Finished runs sit under a day group, so the row drops the date; the project leads
        // the meta line while the list spans every project.
        expect(screen.getByTestId('run-list-day-group')).toHaveTextContent(/^Mar 2\d/)
        expect(screen.getAllByTestId('run-history-row-meta')[0]).toHaveTextContent(/^project-one · Implement Change$/)
        // An untitled run leads with its input and the flow moves to the meta line.
        expect(screen.getAllByTestId('run-history-row-meta')[1]).toHaveTextContent(/^project-one · Merge Change · Failed$/)
        expect(screen.getAllByTestId('run-history-row-status').map((status) => status.textContent)).toEqual([' · Failed'])
        // The dot carries the status; a word appears only for exceptions.
        const rows = screen.getAllByTestId('run-history-row')
        expect(rows[0].querySelector('[data-slot="status-dot"]')).toHaveAttribute('data-tone', 'success')
        expect(rows[1].querySelector('[data-slot="status-dot"]')).toHaveAttribute('data-tone', 'destructive')

        const toggle = screen.getByTestId('run-history-children-toggle')
        expect(toggle).toHaveAccessibleName('Show 1 child run')
        fireEvent.click(toggle)
        expect(titles()).toEqual(['Tighten the review loop', 'Implement Task', 'changes/CR-1/request.md'])
        expect(screen.getAllByTestId('run-history-row-status').map((status) => status.textContent)).toEqual([' · Running', ' · Failed'])
        // A child row drops the date its day group states and the project its parent names,
        // and reads duration · status (its flow is the title).
        expect(screen.getAllByTestId('run-history-row-meta')[1]).toHaveTextContent(/^\d+h \d+m · Running$/)

        fireEvent.change(screen.getByTestId('run-list-search-input'), { target: { value: 'merge' } })
        expect(titles()).toEqual(['changes/CR-1/request.md'])
        fireEvent.change(screen.getByTestId('run-list-search-input'), { target: { value: 'nothing like it' } })
        expect(screen.getByTestId('run-list-search-empty')).toBeVisible()
    })

    it('leaves the project off the rows when the list is filtered to one project, and groups active runs by state', () => {
        render(
            <RunList
                error={null}
                onSelectRun={vi.fn()}
                runs={[
                    makeRun({ run_id: 'waiting', status: 'waiting', title: 'Needs an answer' }),
                    makeRun({ run_id: 'active', status: 'running', title: 'In flight' }),
                    makeRun({ run_id: 'queued', status: 'queued', title: 'Up next' }),
                    makeRun({ run_id: 'old', status: 'completed', title: 'Shipped', started_at: '2026-03-20T12:00:00Z', ended_at: '2026-03-20T12:04:00Z' }),
                ]}
                selectedRunId="active"
                status="ready"
                summaryLabel="4 runs"
                showProject={false}
            />,
        )
        expect(screen.getByTestId('run-list-group-needs-input')).toHaveTextContent(/^Needs input · 1/)
        expect(screen.getByTestId('run-list-group-running')).toHaveTextContent(/^Running · 2/)
        expect(screen.getByTestId('run-list-group-recent')).toHaveTextContent(/^Mar 20Shipped/)
        expect(screen.queryAllByTestId('run-history-row-project')).toHaveLength(0)
        // The group label states the state, so its rows drop the word; a queued run under
        // Running is the exception and keeps it.
        expect(screen.getAllByTestId('run-history-row-meta')[1]).toHaveTextContent(/^Selected · .+ · \d+h \d+m$/)
        expect(screen.getAllByTestId('run-history-row-status').map((status) => status.textContent)).toEqual([' · Queued'])
        expect(screen.getAllByTestId('run-history-row-meta')[3]).toHaveTextContent('Selected · 4m')
        expect(screen.getByTestId('run-list-search-input')).toHaveAttribute('placeholder', 'Search 4 runs')
        const [waiting, active] = screen.getAllByTestId('run-history-row')
        expect(waiting).not.toHaveAttribute('aria-current')
        expect(active).toHaveAttribute('aria-current', 'true')
    })

    it('shows the execution lock in the trailing slot, beside the child-run count, where it cannot truncate', () => {
        const lock = { scope: 'project', key: 'main', conflict_policy: 'queue', identity: 'project:one:main' }
        render(
            <RunList
                error={null}
                onSelectRun={vi.fn()}
                runs={[
                    makeRun({ run_id: 'holder', status: 'running', title: 'A long title that fills the whole row and then some', execution_lock: { ...lock, state: 'holding' } }),
                    makeRun({ run_id: 'child', flow_name: 'Child Flow', parent_run_id: 'holder', root_run_id: 'holder' }),
                    makeRun({ run_id: 'queued', status: 'queued', title: 'Waiting its turn', execution_lock: { ...lock, state: 'queued', queue_position: 2 } }),
                ]}
                selectedRunId={null}
                status="ready"
                summaryLabel="3 runs"
            />,
        )
        const [holding, queued] = screen.getAllByTestId('run-history-row-lock')
        expect(holding).toHaveTextContent('Holding lock')
        expect(holding).toHaveClass('text-warning')
        expect(screen.getByTestId('run-history-children-toggle')).toBeVisible()
        expect(screen.getAllByTestId('run-history-row-meta')[0]).not.toHaveTextContent(/lock/i)
        // Under the lock group the state is already said, so the row keeps only its position.
        expect(queued).toHaveTextContent('Lock queue #2')
        expect(screen.queryAllByTestId('run-history-row-status')).toHaveLength(0)
    })

    it('titles and finds list records by their first launch input', () => {
        const listed = { ...makeRun({ run_id: 'listed', flow_name: 'merge-change.yaml' }), first_launch_input: 'changes/CR-9/request.md' }
        const other = makeRun({ run_id: 'other', title: 'Other work' })
        render(
            <RunList
                error={null}
                onSelectRun={vi.fn()}
                runs={[listed, other]}
                selectedRunId={null}
                status="ready"
                summaryLabel="2 total runs · 2 running"
            />,
        )
        fireEvent.change(screen.getByTestId('run-list-search-input'), { target: { value: 'cr-9' } })
        expect(screen.getAllByTestId('run-history-row-title').map((row) => row.textContent))
            .toEqual(['changes/CR-9/request.md'])
    })

    it('keeps ancestors of a matching child run and reveals the match on search', () => {
        const parent = makeRun({ run_id: 'parent', status: 'completed', title: 'Parent work', flow_name: 'parent.yaml' })
        const child = makeRun({ run_id: 'child', flow_name: 'Middle Child', parent_run_id: 'parent', root_run_id: 'parent' })
        const sibling = makeRun({ run_id: 'sibling', flow_name: 'Other Child', parent_run_id: 'parent', root_run_id: 'parent' })
        const grandchild = makeRun({ run_id: 'grandchild', flow_name: 'Question Child', parent_run_id: 'child', root_run_id: 'parent' })
        render(
            <RunList
                error={null}
                onSelectRun={vi.fn()}
                runs={[parent, child, sibling, grandchild]}
                selectedRunId={null}
                status="ready"
                summaryLabel="4 total runs · 3 running"
            />,
        )
        const titles = () => screen.getAllByTestId('run-history-row-title').map((row) => row.textContent)

        fireEvent.change(screen.getByTestId('run-list-search-input'), { target: { value: 'Question Child' } })
        expect(screen.queryByTestId('run-list-search-empty')).not.toBeInTheDocument()
        expect(titles()).toEqual(['Parent work', 'Middle Child', 'Question Child'])

        fireEvent.change(screen.getByTestId('run-list-search-input'), { target: { value: 'Parent work' } })
        expect(titles()).toEqual(['Parent work'])
        expect(screen.getByTestId('run-history-children-toggle')).toHaveAttribute('title', '2 child runs')
    })
})
