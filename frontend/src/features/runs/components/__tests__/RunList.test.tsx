import { RunList } from '@/features/runs/components/RunList'
import type { RunRecord } from '@/features/runs/model/shared'
import { fireEvent, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { useState } from 'react'
import { UNREACHABLE_MESSAGE } from '@/lib/fetchSpark'
import type { RunStatusKind } from '@/features/runs/model/runOverviewModel'
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
    it('narrows to one state from chips with counts, and finds runs by state words in the search', () => {
        const runs = [
            makeRun({ run_id: 'run-ok', status: 'completed', title: 'Deploy docs', ended_at: '2026-03-22T01:00:00Z' }),
            makeRun({ run_id: 'run-bad', status: 'failed', title: 'Nightly sync', ended_at: '2026-03-22T01:00:00Z' }),
            makeRun({ run_id: 'run-ask', status: 'waiting', title: 'Ship release' }),
        ]
        function Harness() {
            const [statusFilter, setStatusFilter] = useState<RunStatusKind | null>(null)
            return <RunList error={null} onSelectRun={vi.fn()} runs={runs} selectedRunId={null} status="ready" summaryLabel="3 runs" statusFilter={statusFilter} onStatusFilterChange={setStatusFilter} />
        }
        render(<Harness />)
        const ids = () => screen.queryAllByTestId('run-history-row').map((row) => row.dataset.runId)

        // Only states that have runs are offered.
        expect(screen.getByTestId('run-list-status-filter-all')).toHaveTextContent('All3')
        expect(screen.getByTestId('run-list-status-filter-failed')).toHaveTextContent('Failed1')
        expect(screen.queryByTestId('run-list-status-filter-running')).toBeNull()

        fireEvent.click(screen.getByTestId('run-list-status-filter-failed'))
        expect(screen.getByTestId('run-list-status-filter-failed')).toHaveAttribute('aria-pressed', 'true')
        expect(ids()).toEqual(['run-bad'])
        fireEvent.click(screen.getByTestId('run-list-status-filter-all'))
        expect(ids()).toHaveLength(3)

        fireEvent.change(screen.getByTestId('run-list-search-input'), { target: { value: 'failed' } })
        expect(ids()).toEqual(['run-bad'])
        fireEvent.change(screen.getByTestId('run-list-search-input'), { target: { value: 'needs input' } })
        expect(ids()).toEqual(['run-ask'])
    })

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

    it('lists a completed run that reached a failure exit as failed, named by its exit', () => {
        render(
            <RunList
                error={null}
                onSelectRun={vi.fn()}
                runs={[
                    makeRun({ run_id: 'blocked', status: 'completed', outcome: 'failure', outcome_reason_code: 'blocked', title: 'Blocked run' }),
                    makeRun({ run_id: 'shipped', status: 'completed', outcome: 'success', outcome_reason_code: 'completed', title: 'Shipped run' }),
                ]}
                selectedRunId={null}
                status="ready"
                summaryLabel="2 total runs"
            />,
        )
        const tones = screen.getAllByTestId('run-history-row').map((row) => [
            row.getAttribute('data-run-id'),
            row.querySelector('[data-slot="status-dot"]')?.getAttribute('data-tone'),
        ])
        expect(tones).toEqual(expect.arrayContaining([['blocked', 'destructive'], ['shipped', 'success']]))
        expect(screen.getByTestId('run-history-row-status')).toHaveTextContent('Blocked')
    })

    it('keeps a failure exit apart from successes when folding repeats, and counts it as failed', () => {
        const done = (id: string, at: string, outcome: 'success' | 'failure') => makeRun({
            run_id: id, flow_name: 'release.yaml', title: 'Release', status: 'completed', outcome,
            outcome_reason_code: outcome === 'failure' ? 'stop_release' : null, started_at: at, ended_at: at,
        })
        const runs = [done('s1', '2026-03-22T09:00:00Z', 'success'), done('s2', '2026-03-22T08:00:00Z', 'success'), done('x1', '2026-03-22T07:00:00Z', 'failure')]
        render(<RunList error={null} onSelectRun={vi.fn()} runs={runs} selectedRunId={null} status="ready" summaryLabel="3 runs" onStatusFilterChange={vi.fn()} />)
        expect(screen.queryByTestId('run-history-repeats')).toBeNull()
        expect(screen.getByTestId('run-list-status-filter-failed')).toHaveTextContent('Failed1')
        expect(screen.getByTestId('run-history-row-status')).toHaveTextContent('Stop release')
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
        // Finished runs sit under a day group, so the row keeps only the time of day; the
        // project follows it while the list spans every project.
        expect(screen.getByTestId('run-list-day-group')).toHaveTextContent(/^Mar 2\d/)
        expect(screen.getAllByTestId('run-history-row-meta')[0]).toHaveTextContent(/^\d{1,2}:\d{2}\D* · project-one · Implement Change$/)
        // An untitled run leads with its input and the flow moves to the meta line.
        expect(screen.getAllByTestId('run-history-row-meta')[1]).toHaveTextContent(/^\d{1,2}:\d{2}\D* · Failed · project-one · Merge Change$/)
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
        // A child row drops the day its day group states and the project its parent names,
        // and reads time · duration · status (its flow is the title).
        expect(screen.getAllByTestId('run-history-row-meta')[1]).toHaveTextContent(/^\d{1,2}:\d{2}\D* · \d+h \d+m · Running$/)

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
        expect(screen.getAllByTestId('run-history-row-meta')[1]).toHaveTextContent(/^.+ · \d+h \d+m · Selected$/)
        expect(screen.getAllByTestId('run-history-row-status').map((status) => status.textContent)).toEqual([' · Queued'])
        expect(screen.getAllByTestId('run-history-row-meta')[3]).toHaveTextContent(/^\d{1,2}:\d{2}\D* · 4m 00s · Selected$/)
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
    it('folds a day\'s repeats of one run into a row that opens, and keeps the rest in order', () => {
        const done = (id: string, at: string, overrides: Partial<RunRecord> = {}) => makeRun({
            run_id: id, flow_name: 'format-code.yaml', status: 'completed', started_at: at, ended_at: at, ...overrides,
        })
        const runs = [
            done('f1', '2026-03-22T09:00:00Z'),
            done('n1', '2026-03-22T08:30:00Z', { flow_name: 'nightly.yaml', status: 'failed' }),
            done('f2', '2026-03-22T08:00:00Z'),
            done('f3', '2026-03-22T07:00:00Z'),
            done('b1', '2026-03-22T06:00:00Z', { flow_name: 'fix-bug.yaml' }),
            done('b2', '2026-03-22T05:00:00Z', { flow_name: 'fix-bug.yaml' }),
        ]
        const { rerender } = render(<RunList error={null} onSelectRun={vi.fn()} runs={runs} selectedRunId={null} status="ready" summaryLabel="6 runs" />)
        const listed = () => [...screen.getByTestId('run-list-group-recent').querySelectorAll('[data-testid="run-history-row"], [data-testid="run-history-repeats"]')]
            .map((row) => (row as HTMLElement).dataset.runId ?? row.textContent)

        // Two repeats stay rows; three fold into one row at the latest one's place.
        expect(listed()).toEqual([expect.stringContaining('Format Code3 runs'), 'n1', 'b1', 'b2'])
        const repeats = screen.getByTestId('run-history-repeats')
        expect(repeats).toHaveAttribute('aria-expanded', 'false')
        fireEvent.click(repeats)
        expect(listed()).toEqual([expect.stringContaining('Format Code3 runs'), 'f1', 'f2', 'f3', 'n1', 'b1', 'b2'])
        fireEvent.click(repeats)
        expect(listed()).toHaveLength(4)

        // A run opened from elsewhere is shown inside its fold.
        rerender(<RunList error={null} onSelectRun={vi.fn()} runs={runs} selectedRunId="f3" status="ready" summaryLabel="6 runs" />)
        expect(screen.getByTestId('run-history-repeats')).toHaveAttribute('aria-expanded', 'true')
        expect(listed()).toContain('f3')

        // An open fold lists its runs by start time, ten at a time.
        const many = Array.from({ length: 25 }, (_, index) => done(`m${index}`, new Date(Date.parse('2026-03-22T10:00:00Z') + (25 - index) * 1000).toISOString()))
        rerender(<RunList error={null} onSelectRun={vi.fn()} runs={many} selectedRunId={null} status="ready" summaryLabel="25 runs" />)
        fireEvent.click(screen.getByTestId('run-history-repeats'))
        expect(screen.getAllByTestId('run-history-row')).toHaveLength(10)
        expect(new Set(screen.getAllByTestId('run-history-row-title').map((title) => title.textContent)).size).toBe(10)
        fireEvent.click(screen.getByTestId('run-history-repeats-more'))
        expect(screen.getAllByTestId('run-history-row')).toHaveLength(25)
        expect(screen.queryByTestId('run-history-repeats-more')).toBeNull()
    })

    it('opens the parent of a run opened from elsewhere', () => {
        const runs = [
            makeRun({ run_id: 'parent', title: 'Release train', status: 'completed' }),
            makeRun({ run_id: 'child', flow_name: 'child-step.yaml', parent_run_id: 'parent', status: 'completed' }),
        ]
        render(<RunList error={null} onSelectRun={vi.fn()} runs={runs} selectedRunId="child" status="ready" summaryLabel="2 runs" />)
        expect(screen.queryAllByTestId('run-history-row').map((row) => row.dataset.runId)).toContain('child')
    })

    it('is one Tab stop that arrows, j/k, Home and End move through, rows and toggles alike, and Enter opens a run', async () => {
        const user = userEvent.setup()
        const onSelectRun = vi.fn()
        const runs = [
            makeRun({ run_id: 'a', title: 'Alpha', status: 'completed', started_at: '2026-03-22T09:00:00Z' }),
            makeRun({ run_id: 'b', title: 'Beta', status: 'completed', started_at: '2026-03-22T08:00:00Z' }),
            makeRun({ run_id: 'c', title: 'Child', flow_name: 'child.yaml', parent_run_id: 'b', status: 'completed', started_at: '2026-03-22T08:30:00Z' }),
            makeRun({ run_id: 'd', title: 'Delta', status: 'completed', started_at: '2026-03-22T07:00:00Z' }),
        ]
        render(<RunList error={null} onSelectRun={onSelectRun} runs={runs} selectedRunId="d" status="ready" summaryLabel="4 runs" />)
        const region = screen.getByTestId('run-list-scroll-region')
        const row = (id: string) => region.querySelector<HTMLElement>(`[data-run-id="${id}"]`)
        const toggle = screen.getByTestId('run-history-children-toggle')
        const stops = () => [...region.querySelectorAll('button')].filter((button) => button.tabIndex === 0)

        // Tab enters at the open run, and Tab again leaves the list.
        expect(screen.getByRole('navigation', { name: 'Runs' })).toBe(screen.getByTestId('run-list-panel'))
        expect(stops()).toEqual([row('d')])
        await user.click(screen.getByTestId('run-list-search-input'))
        await user.tab()
        expect(row('d')).toHaveFocus()
        await user.tab({ shift: true })
        expect(screen.getByTestId('run-list-search-input')).toHaveFocus()
        await user.tab()

        await user.keyboard('{ArrowUp}')
        expect(toggle).toHaveFocus()
        await user.keyboard('k')
        expect(row('b')).toHaveFocus()
        await user.keyboard('{Home}')
        expect(row('a')).toHaveFocus()
        await user.keyboard('j')
        expect(row('b')).toHaveFocus()
        await user.keyboard('{End}{ArrowDown}')
        expect(row('d')).toHaveFocus()
        expect(onSelectRun).not.toHaveBeenCalled()

        // A toggle still opens its children, which join the list; the stop follows focus.
        await user.keyboard('{ArrowUp}{Enter}{ArrowDown}')
        expect(row('c')).toHaveFocus()
        expect(stops()).toEqual([row('c')])
        await user.keyboard('{Enter}')
        expect(onSelectRun).toHaveBeenCalledWith(expect.objectContaining({ run_id: 'c' }))
    })

    it('says a failed refresh may leave the list out of date, and leaves a failed first load to the main pane', () => {
        const { rerender } = render(<RunList error="Couldn’t load your runs." onSelectRun={vi.fn()} runs={[makeRun()]} selectedRunId={null} status="error" summaryLabel="1 run" />)
        expect(screen.getByTestId('run-list-error')).toHaveTextContent('Couldn’t refresh your runs; this list may be out of date.Try again')

        rerender(<RunList error={UNREACHABLE_MESSAGE} onSelectRun={vi.fn()} runs={[makeRun()]} selectedRunId={null} status="error" summaryLabel="1 run" />)
        expect(screen.getByTestId('run-list-error')).toHaveTextContent('Couldn’t reach Spark; showing the last known runs.Try again')

        rerender(<RunList error="Couldn’t load your runs." onSelectRun={vi.fn()} runs={[]} selectedRunId={null} status="error" summaryLabel="0 runs" />)
        expect(screen.queryByTestId('run-list-error')).toBeNull()
        expect(screen.queryByText('No runs yet.')).toBeNull()
        expect(screen.queryByTestId('run-list-loading')).toBeNull()
    })
})
